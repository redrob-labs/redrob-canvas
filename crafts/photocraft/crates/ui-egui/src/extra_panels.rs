//! Dockable-style floating panels and dialogs that have no home elsewhere: Help › Procedure
//! Browser and Plug-In Browser, Window › Dashboard, Window › Device Status, Window › Selection
//! Editor, Window › MyPaint Brushes, and Image › Metadata › View/Edit Metadata.
//!
//! Every panel here is a floating window (`analysis_ui::panel_window`) toggled by its menu id;
//! the state lives in `UiState::extra` (`ExtraPanels`).

use std::collections::VecDeque;

use egui::{Align2, Color32, ColorImage, FontId, RichText, Sense, TextureOptions, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::analysis_ui::{panel_window, title_row};
use crate::state::Tool;
use crate::theme::Tokens;

pub const PROCEDURE_BROWSER: &str = "help.procedureBrowser";
pub const PLUGIN_BROWSER: &str = "help.pluginBrowser";
pub const DASHBOARD: &str = "window.panel.dashboard";
pub const DEVICE_STATUS: &str = "window.panel.deviceStatus";
pub const SELECTION_EDITOR: &str = "window.panel.selectionEditor";
pub const MYPAINT_BRUSHES: &str = "window.panel.mypaintBrushes";
pub const METADATA_VIEW: &str = "image.metadata.view";
pub const METADATA_EDIT: &str = "image.metadata.edit";

const PANELS: [&str; 6] = [PROCEDURE_BROWSER, PLUGIN_BROWSER, DASHBOARD, DEVICE_STATUS, SELECTION_EDITOR, MYPAINT_BRUSHES];

/// One Dashboard sample.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    /// Seconds since the dashboard started sampling.
    pub t: f64,
    /// Process memory (bytes), when the platform reports it.
    pub rss: u64,
    /// Documents' pixels, history and snapshots (bytes).
    pub documents: u64,
    /// Swap in use (bytes), when known.
    pub swap_used: u64,
    pub swap_total: u64,
    /// UI frame time (ms).
    pub ui_ms: f64,
    /// Last canvas composite + upload (ms).
    pub canvas_ms: f64,
    /// GPU uploads of the last composite.
    pub gpu_uploads: u64,
    /// Running background jobs.
    pub jobs: u32,
}

/// What a device was last using (Device Status).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DeviceRecord {
    pub name: String,
    pub tool: String,
    pub foreground: [f32; 4],
    pub background: [f32; 4],
    pub brush: String,
    pub pattern: String,
    pub gradient: String,
    /// Seconds (app time) the device was last seen.
    pub last_seen: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtraPanels {
    pub procedure_browser: bool,
    pub plugin_browser: bool,
    pub pb_query: String,
    pub pb_selected: Option<String>,
    pub dashboard: bool,
    /// Dashboard › log to file: the path written (the whole history, rewritten every sample).
    pub dashboard_log: Option<String>,
    #[serde(skip)]
    pub samples: VecDeque<Sample>,
    #[serde(skip)]
    pub sample_t0: Option<f64>,
    #[serde(skip)]
    pub last_sample: f64,
    pub device_status: bool,
    pub devices: Vec<DeviceRecord>,
    pub current_device: String,
    pub selection_editor: bool,
    /// Selection Editor › To Path advanced options.
    pub to_path_tolerance: f32,
    pub to_path_threshold: f32,
    pub to_path_advanced: bool,
    pub stroke_width: f32,
    pub mypaint: bool,
    pub mypaint_query: String,
    /// Edit Metadata dialog: the tab shown and the fields being edited.
    pub metadata: Option<(String, Map<String, Value>)>,
    pub metadata_viewer: bool,
    pub metadata_tab: String,
}

const SAMPLE_INTERVAL_S: f64 = 1.0;
const HISTORY: usize = 120;

/// Menu ids handled here.
pub fn handles(id: &str) -> bool {
    PANELS.contains(&id)
}

pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    let e = &app.ui.extra;
    Some(match id {
        PROCEDURE_BROWSER => e.procedure_browser,
        PLUGIN_BROWSER => e.plugin_browser,
        DASHBOARD => e.dashboard,
        DEVICE_STATUS => e.device_status,
        SELECTION_EDITOR => e.selection_editor,
        MYPAINT_BRUSHES => e.mypaint,
        _ => return None,
    })
}

fn no_params(p: &Value) -> bool {
    p.as_object().is_none_or(|o| o.is_empty())
}

/// Toggles a panel, or opens the metadata dialogs. `None` when `id` isn't ours.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let show = params.get("show").and_then(Value::as_bool);
    let e = &mut app.ui.extra;
    let slot = match id {
        PROCEDURE_BROWSER => &mut e.procedure_browser,
        PLUGIN_BROWSER => &mut e.plugin_browser,
        DASHBOARD => &mut e.dashboard,
        DEVICE_STATUS => &mut e.device_status,
        SELECTION_EDITOR => &mut e.selection_editor,
        MYPAINT_BRUSHES => &mut e.mypaint,
        METADATA_VIEW if no_params(params) => {
            app.session.active()?;
            e.metadata_viewer = true;
            return Some(Ok(json!({"dialog": "metadataViewer"})));
        }
        METADATA_EDIT if no_params(params) => {
            let d = app.session.active()?;
            let fields = redrob_photo_engine::metadata_cmds::read_fields(d.doc.metadata.xmp.as_deref());
            e.metadata = Some(("Description".into(), fields));
            return Some(Ok(json!({"dialog": "metadataEditor"})));
        }
        _ => return None,
    };
    *slot = show.unwrap_or(!*slot);
    if id == DASHBOARD && *slot {
        e.sample_t0 = None;
        e.samples.clear();
    }
    Some(Ok(json!({"visible": *slot})))
}

/// Draws the open panels and dialogs; samples the dashboard.
pub fn windows(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.ui.extra.procedure_browser {
        procedure_browser(app, ctx, false);
    }
    if app.ui.extra.plugin_browser {
        procedure_browser(app, ctx, true);
    }
    if app.ui.extra.dashboard {
        dashboard(app, ctx);
    }
    if app.ui.extra.device_status {
        device_status(app, ctx);
    }
    if app.ui.extra.selection_editor {
        selection_editor(app, ctx);
    }
    if app.ui.extra.mypaint {
        mypaint_brushes(app, ctx);
    }
    if app.ui.extra.metadata_viewer {
        metadata_viewer(app, ctx);
    }
    if app.ui.extra.metadata.is_some() {
        metadata_editor(app, ctx);
    }
}

// ------------------------------------------------------------------ Procedure / Plug-In Browser

/// Every command (or, for the Plug-In Browser, only installed plug-ins and the commands with a
/// menu path) matching the query: (id, label, menu path, params, enabled).
fn procedures(app: &PhotocraftApp, query: &str, plugins: bool) -> Vec<(String, String, String, String, bool)> {
    let q = query.trim().to_lowercase();
    let mut out = Vec::new();
    for c in redrob_photo_engine::command_specs() {
        if plugins && c.menu.is_empty() && !c.id.starts_with("plugin.") {
            continue;
        }
        let path = c.menu.join(" › ");
        if !q.is_empty() && !(c.id.to_lowercase().contains(&q) || c.label.to_lowercase().contains(&q) || path.to_lowercase().contains(&q)) {
            continue;
        }
        out.push((c.id.to_string(), c.label.to_string(), path, c.params.to_string(), app.session.is_enabled(c.id)));
    }
    if plugins {
        for p in redrob_photo_plugins::registry::list() {
            let m = p.manifest();
            let id = format!("plugin.filter.{}", m.id);
            if q.is_empty() || id.to_lowercase().contains(&q) || m.name.to_lowercase().contains(&q) {
                out.push((id, m.name.clone(), "Filter › Plug-ins".into(), m.params_notation(), true));
            }
        }
    }
    out.sort_by(|a, b| a.2.cmp(&b.2).then(a.1.cmp(&b.1)));
    out
}

fn procedure_browser(app: &mut PhotocraftApp, ctx: &egui::Context, plugins: bool) {
    let t = Tokens::get(ctx);
    let title = if plugins { tl!("Plug-In Browser") } else { tl!("Procedure Browser") };
    let id = if plugins { "plugin-browser" } else { "procedure-browser" };
    let mut query = app.ui.extra.pb_query.clone();
    let mut selected = app.ui.extra.pb_selected.clone();
    let rows = procedures(app, &query, plugins);
    let mut close = false;
    let mut run: Option<String> = None;
    panel_window(app, ctx, id, title, vec2(-320.0, 60.0), 640.0, |ui| {
        close = title_row(ui, title);
        ui.horizontal(|ui| {
            let icon_rect = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover()).0;
            crate::icons::paint(ui, icon_rect, "search", 12.0, t.text_dim);
            crate::widgets::text_field(ui, egui::Id::new((id, "query")), &mut query, tl!("Search procedures"), 300.0, false);
            ui.label(RichText::new(crate::i18n::fmt(tl!("{n} procedures"), &[("n", &rows.len().to_string())])).color(t.text_faint).size(10.5));
        });
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            egui::ScrollArea::vertical().id_salt((id, "list")).max_height(320.0).max_width(300.0).show(ui, |ui| {
                ui.set_width(296.0);
                let mut last_path = String::new();
                for (cid, label, path, _, enabled) in &rows {
                    if *path != last_path {
                        ui.label(RichText::new(if path.is_empty() { tl!("(no menu)") } else { path.as_str() }).color(t.text_dim).size(10.5).strong());
                        last_path = path.clone();
                    }
                    let on = selected.as_deref() == Some(cid.as_str());
                    let color = if !*enabled {
                        t.text_faint
                    } else if on {
                        t.accent_text
                    } else {
                        t.text
                    };
                    if ui.selectable_label(on, RichText::new(format!("  {label}")).size(11.5).color(color)).clicked() {
                        selected = Some(cid.clone());
                    }
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_width(300.0);
                match selected.as_deref().and_then(|s| rows.iter().find(|r| r.0 == s)) {
                    Some((cid, label, path, params, enabled)) => {
                        ui.label(RichText::new(label.as_str()).color(t.text).size(13.0).strong());
                        ui.label(RichText::new(cid.as_str()).color(t.text_dim).size(11.0).monospace());
                        if !path.is_empty() {
                            ui.label(RichText::new(crate::i18n::fmt(tl!("Menu: {path}"), &[("path", path)])).color(t.text_dim).size(11.0));
                        }
                        ui.label(RichText::new(tl!("Parameters")).color(t.text_dim).size(11.0).strong());
                        egui::ScrollArea::vertical().id_salt((id, "params")).max_height(200.0).show(ui, |ui| {
                            ui.label(RichText::new(params.as_str()).size(10.5).monospace().color(t.text));
                        });
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(if *enabled { tl!("Available now") } else { tl!("Not available in the current state") })
                                .color(t.text_faint)
                                .size(10.5),
                        );
                        if ui.add_enabled(*enabled && !cid.starts_with("plugin.filter."), egui::Button::new(tl!("Run"))).clicked() {
                            run = Some(cid.clone());
                        }
                    }
                    None => {
                        ui.label(RichText::new(tl!("Select a procedure to read its parameters.")).color(t.text_faint).size(11.0));
                    }
                }
            });
        });
    });
    app.ui.extra.pb_query = query;
    app.ui.extra.pb_selected = selected;
    if close {
        if plugins {
            app.ui.extra.plugin_browser = false;
        } else {
            app.ui.extra.procedure_browser = false;
        }
    }
    if let Some(id) = run {
        let _ = crate::menus::invoke(app, ctx, &id, json!({}));
    }
}

// ------------------------------------------------------------------ Dashboard

#[cfg(target_os = "linux")]
fn process_rss() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = s.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096)
}

#[cfg(not(target_os = "linux"))]
fn process_rss() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn swap() -> Option<(u64, u64)> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb = |key: &str| -> Option<u64> { info.lines().find_map(|l| l.strip_prefix(key))?.trim().trim_end_matches("kB").trim().parse().ok() };
    let total = kb("SwapTotal:")? * 1024;
    let free = kb("SwapFree:")? * 1024;
    Some((total.saturating_sub(free), total))
}

#[cfg(not(target_os = "linux"))]
fn swap() -> Option<(u64, u64)> {
    None
}

/// Takes a sample when the interval has passed; returns true when one was taken.
pub fn sample(app: &mut PhotocraftApp, now_s: f64) -> bool {
    let e = &mut app.ui.extra;
    let t0 = *e.sample_t0.get_or_insert(now_s);
    if !e.samples.is_empty() && now_s - e.last_sample < SAMPLE_INTERVAL_S {
        return false;
    }
    e.last_sample = now_s;
    let documents: u64 = app.session.documents().iter().map(|d| d.memory_bytes() as u64).sum();
    let (swap_used, swap_total) = swap().unwrap_or((0, 0));
    let s = Sample {
        t: now_s - t0,
        rss: process_rss().unwrap_or(0),
        documents,
        swap_used,
        swap_total,
        ui_ms: app.perf.ui_ms,
        canvas_ms: app.perf.composite_ms + app.perf.upload_ms,
        gpu_uploads: app.perf.gpu_uploads,
        jobs: app.session.jobs().len() as u32,
    };
    let e = &mut app.ui.extra;
    e.samples.push_back(s);
    while e.samples.len() > HISTORY {
        e.samples.pop_front();
    }
    true
}

fn mb(bytes: u64) -> String {
    if bytes >= 1 << 30 { format!("{:.2} GB", bytes as f64 / f64::from(1u32 << 30)) } else { format!("{:.1} MB", bytes as f64 / f64::from(1u32 << 20)) }
}

/// The dashboard log as CSV (one row per sample).
pub fn log_csv(samples: &VecDeque<Sample>) -> String {
    let mut s = String::from("t_s,rss_bytes,documents_bytes,swap_used_bytes,swap_total_bytes,ui_ms,canvas_ms,gpu_uploads,jobs\n");
    for x in samples {
        s.push_str(&format!(
            "{:.1},{},{},{},{},{:.2},{:.2},{},{}\n",
            x.t, x.rss, x.documents, x.swap_used, x.swap_total, x.ui_ms, x.canvas_ms, x.gpu_uploads, x.jobs
        ));
    }
    s
}

/// Low memory: swap more than 90 % used, or the process past 90 % of physical memory.
pub fn low_memory(s: &Sample) -> bool {
    (s.swap_total > 0 && s.swap_used as f64 > 0.9 * s.swap_total as f64)
        || crate::gpu_canvas::physical_memory().is_some_and(|ram| s.rss > 0 && s.rss as f64 > 0.9 * ram as f64)
}

fn graph(ui: &mut egui::Ui, title: &str, now: &str, values: &[f64], max: f64, color: Color32) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).color(t.text_dim).size(11.0).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(now).color(t.text).size(11.0));
        });
    });
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
    let p = ui.painter_at(r);
    p.rect_filled(r, 2.0, t.card);
    p.rect_stroke(r, 2.0, egui::Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
    if values.len() >= 2 && max > 0.0 {
        let n = HISTORY.max(2) as f32;
        let pts: Vec<egui::Pos2> = values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let x = r.right() - (values.len() - 1 - i) as f32 * (r.width() / (n - 1.0));
                let y = r.bottom() - ((*v / max).clamp(0.0, 1.0) as f32) * (r.height() - 2.0) - 1.0;
                pos2(x, y)
            })
            .collect();
        p.add(egui::Shape::line(pts, egui::Stroke::new(1.5, color)));
    }
    ui.add_space(4.0);
}

fn dashboard(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let now = crate::gpu_canvas::now_ms() / 1000.0;
    let took = sample(app, now);
    if took && let Some(path) = app.ui.extra.dashboard_log.clone() {
        let csv = log_csv(&app.ui.extra.samples);
        if let Some(write) = app.services.write.as_mut()
            && let Err(e) = write(&path, csv.as_bytes())
        {
            app.ui.status = e;
            app.ui.status_error = true;
            app.ui.extra.dashboard_log = None;
        }
    }
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(SAMPLE_INTERVAL_S));
    let t = Tokens::get(ctx);
    let samples: Vec<Sample> = app.ui.extra.samples.iter().copied().collect();
    let last = samples.last().copied().unwrap_or_default();
    let ram = crate::gpu_canvas::physical_memory();
    let gpu_budget = app.perf.gpu_budget_mb;
    let gpu_on = app.perf.gpu;
    let history_budget = app.session.prefs().performance.history_budget_bytes() as u64;
    let logging = app.ui.extra.dashboard_log.clone();
    let mut close = false;
    let mut toggle_log = false;
    let mut reset = false;
    panel_window(app, ctx, "dashboard", tl!("Dashboard"), vec2(-20.0, 60.0), 300.0, |ui| {
        close = title_row(ui, tl!("Dashboard"));
        if low_memory(&last) {
            ui.label(RichText::new(tl!("Low memory: swap is nearly full. Save your work.")).color(t.warning).size(11.0));
        }
        let col = |v: &dyn Fn(&Sample) -> f64| samples.iter().map(v).collect::<Vec<f64>>();
        let mem = col(&|s| s.rss as f64);
        let mem_max = ram.map_or_else(|| mem.iter().cloned().fold(1.0, f64::max), |r| r as f64);
        graph(ui, tl!("Memory"), &format!("{}{}", mb(last.rss), ram.map(|r| format!(" / {}", mb(r))).unwrap_or_default()), &mem, mem_max, t.accent);
        let docs = col(&|s| s.documents as f64);
        graph(ui, tl!("Cache"), &format!("{} / {}", mb(last.documents), mb(history_budget)), &docs, history_budget.max(1) as f64, t.accent_soft);
        if last.swap_total > 0 {
            graph(
                ui,
                tl!("Swap"),
                &format!("{} / {}", mb(last.swap_used), mb(last.swap_total)),
                &col(&|s| s.swap_used as f64),
                last.swap_total as f64,
                t.warning,
            );
        } else {
            ui.label(RichText::new(tl!("Swap: not reported on this platform")).color(t.text_faint).size(10.5));
        }
        let cpu = col(&|s| s.ui_ms + s.canvas_ms);
        graph(ui, tl!("CPU"), &format!("{:.1} ms", last.ui_ms + last.canvas_ms), &cpu, 33.0, t.text);
        let gpu = col(&|s| s.gpu_uploads as f64);
        let gpu_max = gpu.iter().cloned().fold(1.0, f64::max);
        graph(
            ui,
            tl!("GPU"),
            &if gpu_on { format!("{} {} · {} MB", last.gpu_uploads, tl!("uploads"), gpu_budget) } else { tl!("CPU renderer").to_string() },
            &gpu,
            gpu_max,
            t.accent,
        );
        ui.label(
            RichText::new(crate::i18n::fmt(
                tl!("Misc: {jobs} jobs running · {n} samples"),
                &[("jobs", &last.jobs.to_string()), ("n", &samples.len().to_string())],
            ))
            .color(t.text_dim)
            .size(10.5),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let label = if logging.is_some() { tl!("Stop Logging") } else { tl!("Log to File…") };
            if crate::widgets::secondary_button(ui, label, 0.0).clicked() {
                toggle_log = true;
            }
            if crate::widgets::secondary_button(ui, tl!("Reset"), 0.0).clicked() {
                reset = true;
            }
        });
        if let Some(p) = &logging {
            ui.label(RichText::new(p.as_str()).color(t.text_faint).size(10.0));
        }
    });
    if close {
        app.ui.extra.dashboard = false;
    }
    if reset {
        app.ui.extra.samples.clear();
        app.ui.extra.sample_t0 = None;
    }
    if toggle_log {
        if app.ui.extra.dashboard_log.is_some() {
            app.ui.extra.dashboard_log = None;
        } else {
            let r = app.pick_save("Dashboard log.csv", |app, path| {
                let csv = log_csv(&app.ui.extra.samples);
                let write = app.services.write.as_mut().ok_or("no writer configured")?;
                write(&path, csv.as_bytes())?;
                app.ui.extra.dashboard_log = Some(path.clone());
                Ok(json!({"path": path}))
            });
            if let Err(e) = r
                && e != "cancelled"
            {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
    }
}

// ------------------------------------------------------------------ Device Status

fn tool_name(t: Tool) -> String {
    let s = format!("{t:?}");
    let mut c = s.chars();
    c.next().map(|f| f.to_ascii_lowercase().to_string() + c.as_str()).unwrap_or_default()
}

fn capture(app: &PhotocraftApp, name: &str, now: f64) -> DeviceRecord {
    DeviceRecord {
        name: name.to_string(),
        tool: tool_name(app.ui.tool),
        foreground: app.session.tools.foreground,
        background: app.session.tools.background,
        brush: app.session.tools.current_preset.clone().unwrap_or_default(),
        pattern: app.session.presets.pattern.clone().unwrap_or_default(),
        gradient: app.session.presets.gradient.name.clone(),
        last_seen: now,
    }
}

/// Applies a device's remembered status: tool, colours, brush, pattern and gradient.
fn apply(app: &mut PhotocraftApp, r: &DeviceRecord) {
    if let Some(tool) = Tool::from_name(&r.tool) {
        crate::tool_memory::select(app, tool);
    }
    let _ = app.run("tools.setColors", json!({"foreground": r.foreground, "background": r.background}));
    if !r.brush.is_empty() {
        let _ = app.run("brush.presets.setCurrent", json!({"name": r.brush}));
    }
    if !r.pattern.is_empty() {
        let _ = app.run("pattern.presets.select", json!({"pattern": r.pattern, "applyToLayer": false}));
    }
    if !r.gradient.is_empty() {
        let _ = app.run("gradient.presets.select", json!({"preset": r.gradient}));
    }
}

/// Loads the saved device status from the preferences the first time anything asks.
fn load_devices(app: &mut PhotocraftApp) {
    if !app.ui.extra.devices.is_empty() {
        return;
    }
    if let Some(v) = app.session.prefs().dialogs.get("deviceStatus")
        && let Ok(list) = serde_json::from_value::<Vec<DeviceRecord>>(v.clone())
    {
        app.ui.extra.devices = list;
    }
}

/// Called with every canvas pointer press: tells pens (a pressure other than full) from the
/// mouse, keeps each device's status and switches to the newly active device's status.
pub fn device_seen(app: &mut PhotocraftApp, ev: &crate::canvas::ToolEvent) {
    let crate::canvas::ToolEvent::Down { pressure, .. } = ev else { return };
    // Devices are tracked once the panel was opened or a status was saved; until then a pen and
    // the mouse share one state, as they always did.
    if !app.ui.extra.device_status && app.ui.extra.devices.is_empty() && !app.session.prefs().dialogs.contains_key("deviceStatus") {
        return;
    }
    let name = if (*pressure - 1.0).abs() > 1e-3 && *pressure > 0.0 { "Pen" } else { "Mouse" };
    device_switch(app, name);
}

/// Switches the active input device: the old one's status is remembered, the new one's restored.
pub fn device_switch(app: &mut PhotocraftApp, name: &str) {
    load_devices(app);
    let now = crate::gpu_canvas::now_ms() / 1000.0;
    if app.ui.extra.current_device == name {
        let rec = capture(app, name, now);
        match app.ui.extra.devices.iter_mut().find(|d| d.name == name) {
            Some(d) => *d = rec,
            None => app.ui.extra.devices.push(rec),
        }
        return;
    }
    let old = app.ui.extra.current_device.clone();
    if !old.is_empty() {
        let rec = capture(app, &old, now);
        match app.ui.extra.devices.iter_mut().find(|d| d.name == old) {
            Some(d) => *d = rec,
            None => app.ui.extra.devices.push(rec),
        }
    }
    app.ui.extra.current_device = name.to_string();
    match app.ui.extra.devices.iter().find(|d| d.name == name).cloned() {
        Some(mut r) => {
            apply(app, &r);
            r.last_seen = now;
            if let Some(d) = app.ui.extra.devices.iter_mut().find(|d| d.name == name) {
                *d = r;
            }
        }
        None => {
            let rec = capture(app, name, now);
            app.ui.extra.devices.push(rec);
        }
    }
}

/// Save Device Status: the records go to the preferences.
pub fn save_devices(app: &mut PhotocraftApp) -> Result<Value, String> {
    let now = crate::gpu_canvas::now_ms() / 1000.0;
    let cur = app.ui.extra.current_device.clone();
    if !cur.is_empty() {
        let rec = capture(app, &cur, now);
        match app.ui.extra.devices.iter_mut().find(|d| d.name == cur) {
            Some(d) => *d = rec,
            None => app.ui.extra.devices.push(rec),
        }
    }
    let v = serde_json::to_value(&app.ui.extra.devices).unwrap_or_default();
    app.session.edit_prefs(|p| {
        p.dialogs.insert("deviceStatus".into(), v);
    });
    crate::prefs_ui::save_preferences(app)?;
    Ok(json!({"devices": app.ui.extra.devices.len()}))
}

fn swatch(ui: &mut egui::Ui, c: [f32; 4]) {
    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    ui.painter().rect_filled(r, 2.0, Color32::from_rgb(q(c[0]), q(c[1]), q(c[2])));
    ui.painter().rect_stroke(r, 2.0, egui::Stroke::new(1.0, Tokens::get(ui.ctx()).card_border), egui::StrokeKind::Inside);
}

fn device_status(app: &mut PhotocraftApp, ctx: &egui::Context) {
    load_devices(app);
    if app.ui.extra.current_device.is_empty() {
        device_switch(app, "Mouse");
    }
    let t = Tokens::get(ctx);
    let now = crate::gpu_canvas::now_ms() / 1000.0;
    let cur = app.ui.extra.current_device.clone();
    let mut devices = app.ui.extra.devices.clone();
    if let Some(d) = devices.iter_mut().find(|d| d.name == cur) {
        *d = capture(app, &cur, d.last_seen.max(now - 1.0));
    }
    let mut close = false;
    let mut save = false;
    let mut pick: Option<String> = None;
    panel_window(app, ctx, "device-status", tl!("Device Status"), vec2(-20.0, 400.0), 300.0, |ui| {
        close = title_row(ui, tl!("Device Status"));
        for d in &devices {
            let active = d.name == cur;
            egui::Frame::NONE.fill(if active { t.row_selected } else { Color32::TRANSPARENT }).corner_radius(3.0).inner_margin(egui::Margin::same(4)).show(
                ui,
                |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let icon = if d.name == "Pen" { "pen-tool" } else { "mouse-pointer" };
                        if crate::icons::exists(icon) {
                            let icon_rect = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover()).0;
                            crate::icons::paint(ui, icon_rect, icon, 13.0, t.icon);
                        }
                        let shown = match d.name.as_str() {
                            "Pen" => tl!("Pen / Tablet"),
                            "Mouse" => tl!("Core Pointer"),
                            other => other,
                        };
                        if ui.selectable_label(active, RichText::new(shown).size(12.0).strong().color(t.text)).clicked() && !active {
                            pick = Some(d.name.clone());
                        }
                    });
                    ui.horizontal(|ui| {
                        let tool = Tool::from_name(&d.tool).map_or(d.tool.clone(), |t| tl!(t.label()).to_string());
                        if let Some(icon) = Tool::from_name(&d.tool).map(crate::icons::tool_icon) {
                            let icon_rect = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover()).0;
                            crate::icons::paint(ui, icon_rect, icon, 13.0, t.icon);
                        }
                        ui.label(RichText::new(tool).size(11.0).color(t.text));
                        swatch(ui, d.foreground);
                        swatch(ui, d.background);
                    });
                    let line = |ui: &mut egui::Ui, label: &str, v: &str| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(label).color(t.text_dim).size(10.5));
                            ui.label(RichText::new(if v.is_empty() { "—" } else { v }).color(t.text).size(10.5));
                        });
                    };
                    line(ui, tl!("Brush:"), &d.brush);
                    line(ui, tl!("Pattern:"), &d.pattern);
                    line(ui, tl!("Gradient:"), &d.gradient);
                },
            );
            ui.add_space(2.0);
        }
        ui.add_space(4.0);
        if crate::widgets::secondary_button(ui, tl!("Save Device Status"), 0.0).clicked() {
            save = true;
        }
        ui.label(
            RichText::new(tl!("Each device remembers its tool, colours, brush, pattern and gradient; switching devices switches them."))
                .color(t.text_faint)
                .size(10.0),
        );
    });
    if close {
        app.ui.extra.device_status = false;
    }
    if let Some(name) = pick {
        device_switch(app, &name);
    }
    if save && let Err(e) = save_devices(app) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

// ------------------------------------------------------------------ Selection Editor

/// The selection as a grayscale image at most `max` px wide (white = selected).
pub fn selection_image(doc: &redrob_photo_doc::Document, max: usize) -> ColorImage {
    let (w, h) = (doc.size.width.max(1) as usize, doc.size.height.max(1) as usize);
    let k = (w.max(h) as f32 / max as f32).max(1.0);
    let (ow, oh) = (((w as f32 / k) as usize).max(1), ((h as f32 / k) as usize).max(1));
    let mut img = ColorImage::new([ow, oh], vec![Color32::BLACK; ow * oh]);
    if let Some(sel) = &doc.selection {
        for y in 0..oh {
            for x in 0..ow {
                let v = sel.sample_channel((x as f32 * k) as i32, (y as f32 * k) as i32, 0).clamp(0.0, 1.0);
                let g = (v * 255.0).round() as u8;
                img.pixels[y * ow + x] = Color32::from_gray(g);
            }
        }
    }
    img
}

fn selection_editor(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let t = Tokens::get(ctx);
    let Some((doc, rev)) = app.session.active().map(|d| (d.doc.clone(), d.history.past_len())) else {
        let mut close = false;
        panel_window(app, ctx, "selection-editor", tl!("Selection Editor"), vec2(-20.0, 420.0), 260.0, |ui| {
            close = title_row(ui, tl!("Selection Editor"));
            ui.label(RichText::new(tl!("No document open.")).color(t.text_faint));
        });
        if close {
            app.ui.extra.selection_editor = false;
        }
        return;
    };
    let key = egui::Id::new(("selection-editor-tex", doc.id.0));
    let sel_key = (rev, doc.selection.as_ref().map(|s| s.content_bounds()), doc.size);
    let cached: Option<(TexKey, egui::TextureHandle)> = ctx.data(|d| d.get_temp(key));
    let tex = match cached.filter(|(k, _)| *k == sel_key) {
        Some((_, t)) => t,
        None => {
            let img = selection_image(&doc, 240);
            let tex = ctx.load_texture("selection-editor", img, TextureOptions::NEAREST);
            ctx.data_mut(|d| d.insert_temp(key, (sel_key, tex.clone())));
            tex
        }
    };
    let mut close = false;
    let mut act: Option<(&str, Value)> = None;
    let mut clicked: Option<[f64; 2]> = None;
    let mut tol = app.ui.extra.to_path_tolerance.max(0.5);
    let mut thr = app.ui.extra.to_path_threshold.max(1.0);
    let mut adv = app.ui.extra.to_path_advanced;
    let mut width = app.ui.extra.stroke_width.max(1.0);
    let has_sel = doc.selection.is_some();
    panel_window(app, ctx, "selection-editor", tl!("Selection Editor"), vec2(-20.0, 420.0), 260.0, |ui| {
        close = title_row(ui, tl!("Selection Editor"));
        let size = tex.size_vec2();
        let scale = (240.0 / size.x).min(240.0 / size.y.max(1.0)).min(1.0);
        let (r, resp) = ui.allocate_exact_size(size * scale, Sense::click());
        ui.painter().image(tex.id(), r, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
        if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            let fx = ((p.x - r.left()) / r.width()).clamp(0.0, 1.0);
            let fy = ((p.y - r.top()) / r.height()).clamp(0.0, 1.0);
            clicked = Some([f64::from(fx) * f64::from(doc.size.width), f64::from(fy) * f64::from(doc.size.height)]);
        }
        resp.on_hover_text(tl!("Click to select the contiguous colour there with the Magic Wand (Shift adds, Alt subtracts)"));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if crate::widgets::secondary_button(ui, tl!("All"), 0.0).clicked() {
                act = Some(("select.all", json!({})));
            }
            if ui.add_enabled(has_sel, egui::Button::new(tl!("None"))).clicked() {
                act = Some(("select.deselect", json!({})));
            }
            if ui.add_enabled(has_sel, egui::Button::new(tl!("Invert"))).clicked() {
                act = Some(("select.inverse", json!({})));
            }
        });
        ui.horizontal(|ui| {
            if ui.add_enabled(has_sel, egui::Button::new(tl!("Save to Channel"))).clicked() {
                act = Some(("select.saveSelection", json!({})));
            }
            if ui.add_enabled(has_sel, egui::Button::new(tl!("To Path"))).clicked() {
                act = Some(("select.toWorkPath", json!({"tolerance": tol, "threshold": thr})));
            }
            if ui.add_enabled(has_sel, egui::Button::new(tl!("Stroke"))).clicked() {
                act = Some(("edit.stroke", json!({"width": width})));
            }
        });
        crate::widgets::checkbox(ui, &mut adv, tl!("Advanced options"));
        if adv {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Tolerance")).color(t.text_dim).size(10.5));
                crate::widgets::value_field(ui, &mut tol, 0.5..=10.0, "px", 56.0);
                ui.label(RichText::new(tl!("Threshold")).color(t.text_dim).size(10.5));
                crate::widgets::value_field(ui, &mut thr, 1.0..=99.0, "%", 56.0);
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Stroke width")).color(t.text_dim).size(10.5));
                crate::widgets::value_field(ui, &mut width, 1.0..=250.0, "px", 56.0);
            });
        }
    });
    app.ui.extra.to_path_tolerance = tol;
    app.ui.extra.to_path_threshold = thr;
    app.ui.extra.to_path_advanced = adv;
    app.ui.extra.stroke_width = width;
    if close {
        app.ui.extra.selection_editor = false;
    }
    if let Some(p) = clicked {
        let mods = ctx.input(|i| i.modifiers);
        let mode = if mods.shift {
            "add"
        } else if mods.alt {
            "subtract"
        } else {
            "replace"
        };
        let o = &app.ui.tool_options;
        let params = json!({"x": p[0].floor(), "y": p[1].floor(), "tolerance": o.tolerance, "contiguous": true, "antiAlias": o.anti_alias, "sampleAllLayers": true, "criterion": o.wand_criterion, "mode": mode});
        let _ = app.run("select.magicWand", params);
    }
    if let Some((id, p)) = act {
        let _ = crate::menus::invoke(app, ctx, id, p);
    }
}

type TexKey = (usize, Option<redrob_photo_geom::Rect>, redrob_photo_geom::Size);

// ------------------------------------------------------------------ MyPaint Brushes

fn mypaint_brushes(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let t = Tokens::get(ctx);
    let mut query = app.ui.extra.mypaint_query.clone();
    let brushes = redrob_photo_engine::mypaint_cmds::list_brushes(&app.session);
    let q = query.trim().to_lowercase();
    let rows: Vec<&Value> = brushes
        .iter()
        .filter(|b| {
            q.is_empty()
                || b["name"].as_str().is_some_and(|n| n.to_lowercase().contains(&q))
                || b["tags"].as_array().is_some_and(|tags| tags.iter().any(|x| x.as_str().is_some_and(|x| x.to_lowercase().starts_with(&q))))
        })
        .collect();
    let current = app.session.tools.current_preset.clone().unwrap_or_default();
    let mut close = false;
    let mut pick: Option<String> = None;
    let mut import = false;
    let mut tag: Option<(String, String)> = None;
    panel_window(app, ctx, "mypaint-brushes", tl!("MyPaint Brushes"), vec2(-20.0, 80.0), 300.0, |ui| {
        close = title_row(ui, tl!("MyPaint Brushes"));
        ui.horizontal(|ui| {
            let icon_rect = ui.allocate_exact_size(vec2(14.0, 18.0), Sense::hover()).0;
            crate::icons::paint(ui, icon_rect, "search", 12.0, t.text_dim);
            crate::widgets::text_field(ui, egui::Id::new("mypaint-filter"), &mut query, tl!("Filter by tag"), 200.0, false);
            if crate::widgets::secondary_button(ui, tl!("Import…"), 0.0).clicked() {
                import = true;
            }
        });
        if rows.is_empty() {
            ui.label(
                RichText::new(if brushes.is_empty() {
                    tl!("No MyPaint brushes yet. Import .myb files to list them here.")
                } else {
                    tl!("No brush matches the filter.")
                })
                .color(t.text_faint)
                .size(11.0),
            );
        }
        egui::ScrollArea::vertical().id_salt("mypaint-list").max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
            for b in &rows {
                let name = b["name"].as_str().unwrap_or_default().to_string();
                let tags: Vec<String> = b["tags"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
                let on = name == current;
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                if on {
                    ui.painter().rect_filled(r, 2.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 2.0, t.hover.gamma_multiply(0.5));
                }
                let size = b["size"].as_f64().unwrap_or(10.0) as f32;
                let dot = (size / 4.0).clamp(2.0, 11.0);
                let c = if b["eraser"].as_bool().unwrap_or(false) { t.text_faint } else { t.text };
                ui.painter().circle_filled(pos2(r.left() + 14.0, r.center().y), dot, c);
                ui.painter().text(pos2(r.left() + 30.0, r.top() + 9.0), Align2::LEFT_CENTER, &name, FontId::proportional(11.5), t.text);
                ui.painter().text(pos2(r.left() + 30.0, r.bottom() - 8.0), Align2::LEFT_CENTER, tags.join(", "), FontId::proportional(10.0), t.text_dim);
                if resp.clicked() {
                    pick = Some(name.clone());
                }
                resp.context_menu(|ui| {
                    ui.label(RichText::new(tl!("Add tag")).color(t.text_dim).size(10.5));
                    let id = egui::Id::new(("mypaint-tag", name.clone()));
                    let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_default();
                    let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(120.0));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !text.trim().is_empty() {
                        tag = Some((name.clone(), text.trim().to_string()));
                        text.clear();
                        ui.close();
                    }
                    ui.data_mut(|d| d.insert_temp(id, text));
                });
            }
        });
    });
    app.ui.extra.mypaint_query = query;
    if close {
        app.ui.extra.mypaint = false;
    }
    if let Some(name) = pick {
        let _ = app.run("brush.presets.setCurrent", json!({"name": name}));
    }
    if let Some((name, tag)) = tag {
        let _ = app.run("brush.presets.tag", json!({"name": name, "tag": tag}));
    }
    if import {
        let r = app.pick_file_bytes_filtered(&["myb"], |app, name, bytes| {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let file = std::path::Path::new(&name).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or(name);
            app.run("brush.mypaint.import", json!({"name": file, "text": text}))
        });
        if let Err(e) = r
            && e != "cancelled"
        {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

// ------------------------------------------------------------------ Metadata viewer / editor

fn metadata_viewer(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let t = Tokens::get(ctx);
    let Ok(v) = app.run("image.metadata.view", json!({})) else {
        app.ui.extra.metadata_viewer = false;
        return;
    };
    let mut tab = if app.ui.extra.metadata_tab.is_empty() { "exif".to_string() } else { app.ui.extra.metadata_tab.clone() };
    let mut close = false;
    panel_window(app, ctx, "metadata-viewer", tl!("Metadata Viewer"), vec2(-300.0, 60.0), 560.0, |ui| {
        close = title_row(ui, tl!("Metadata Viewer"));
        ui.horizontal(|ui| {
            for (k, label, n) in [
                ("exif", tl!("EXIF"), v["exif"].as_array().map_or(0, Vec::len)),
                ("xmp", tl!("XMP"), v["xmp"].as_array().map_or(0, Vec::len)),
                ("iptc", tl!("IPTC"), v["iptc"].as_array().map_or(0, Vec::len)),
                ("text", tl!("Text"), v["text"].as_array().map_or(0, Vec::len)),
            ] {
                if ui.selectable_label(tab == k, format!("{label} ({n})")).clicked() {
                    tab = k.to_string();
                }
            }
        });
        crate::widgets::hairline(ui);
        let rows = v[tab.as_str()].as_array().cloned().unwrap_or_default();
        egui::ScrollArea::vertical().id_salt("metadata-viewer-rows").max_height(360.0).show(ui, |ui| {
            egui::Grid::new("metadata-viewer-grid").striped(true).num_columns(2).spacing(vec2(12.0, 2.0)).show(ui, |ui| {
                for r in &rows {
                    let (name, value) = match tab.as_str() {
                        "exif" => {
                            let n = r["name"].as_str().unwrap_or_default();
                            (
                                if n.is_empty() {
                                    format!("{} {}", r["ifd"].as_str().unwrap_or_default(), r["tag"].as_str().unwrap_or_default())
                                } else {
                                    n.to_string()
                                },
                                r["value"].as_str().unwrap_or_default().to_string(),
                            )
                        }
                        "xmp" => (r["property"].as_str().unwrap_or_default().to_string(), r["value"].as_str().unwrap_or_default().to_string()),
                        "iptc" => {
                            let n = r["name"].as_str().unwrap_or_default();
                            (
                                if n.is_empty() { format!("{}:{}", r["record"], r["dataset"]) } else { n.to_string() },
                                r["value"].as_str().unwrap_or_default().to_string(),
                            )
                        }
                        _ => (r["key"].as_str().unwrap_or_default().to_string(), r["value"].as_str().unwrap_or_default().to_string()),
                    };
                    ui.label(RichText::new(name).color(t.text_dim).size(10.5));
                    ui.add(egui::Label::new(RichText::new(value).color(t.text).size(10.5)).wrap());
                    ui.end_row();
                }
                if rows.is_empty() {
                    ui.label(RichText::new(tl!("No tags")).color(t.text_faint).size(10.5));
                    ui.end_row();
                }
            });
        });
    });
    app.ui.extra.metadata_tab = tab;
    if close {
        app.ui.extra.metadata_viewer = false;
    }
}

fn metadata_editor(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some((mut tab, mut fields)) = app.ui.extra.metadata.clone() else { return };
    let t = Tokens::get(ctx);
    let mut result: Option<bool> = None;
    egui::Window::new(tl!("Edit Metadata"))
        .id(egui::Id::new("metadata-editor"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(520.0);
            ui.horizontal_wrapped(|ui| {
                for tb in redrob_photo_engine::metadata_cmds::TABS {
                    if ui.selectable_label(tab == tb, tl!(tb)).clicked() {
                        tab = tb.to_string();
                    }
                }
            });
            crate::widgets::hairline(ui);
            ui.add_space(4.0);
            egui::ScrollArea::vertical().id_salt("metadata-editor-fields").max_height(360.0).show(ui, |ui| {
                egui::Grid::new("metadata-editor-grid").num_columns(2).spacing(vec2(10.0, 6.0)).min_col_width(150.0).show(ui, |ui| {
                    for (key, label, ftab, _, kind) in redrob_photo_engine::metadata_cmds::FIELDS {
                        if *ftab != tab {
                            continue;
                        }
                        ui.label(RichText::new(tl!(label)).color(t.text_dim).size(11.0));
                        let mut v = fields.get(*key).and_then(Value::as_str).unwrap_or("").to_string();
                        let changed = if *key == "copyrightStatus" {
                            crate::widgets::dropdown(
                                ui,
                                "md-copyright-status",
                                &mut v,
                                &[
                                    ("unknown".to_string(), tl!("Unknown")),
                                    ("copyrighted".to_string(), tl!("Copyrighted")),
                                    ("publicDomain".to_string(), tl!("Public Domain")),
                                ],
                                160.0,
                            )
                        } else if *key == "description" || *key == "instructions" || *key == "usageTerms" {
                            ui.add(egui::TextEdit::multiline(&mut v).desired_rows(2).desired_width(320.0)).changed()
                        } else {
                            ui.add(egui::TextEdit::singleline(&mut v).desired_width(320.0)).changed()
                        };
                        if changed {
                            fields.insert((*key).into(), json!(v));
                        }
                        if *kind == "Bag" || *kind == "Seq" {
                            ui.end_row();
                            ui.label("");
                            ui.label(RichText::new(tl!("Separate several values with semicolons")).color(t.text_faint).size(9.5));
                        }
                        ui.end_row();
                    }
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    use crate::widgets::{ButtonRole, DialogButton};
                    if let Some(role) = crate::widgets::dialog_buttons(
                        ui,
                        &[DialogButton::new(ButtonRole::Default, tl!("Write Metadata"), 120.0), DialogButton::new(ButtonRole::Cancel, tl!("Cancel"), 80.0)],
                    ) {
                        result = Some(role == ButtonRole::Default);
                    }
                });
            });
        });
    app.ui.extra.metadata = Some((tab, fields.clone()));
    match result {
        Some(true) => {
            if app.run("image.metadata.edit", Value::Object(fields)).is_ok() {
                app.ui.extra.metadata = None;
            }
        }
        Some(false) => app.ui.extra.metadata = None,
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(redrob_photo_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        app
    }

    #[test]
    fn panels_toggle_and_procedure_search_filters() {
        let mut app = app();
        assert_eq!(checked(&app, DASHBOARD), Some(false));
        menu(&mut app, DASHBOARD, &json!({})).unwrap().unwrap();
        assert_eq!(checked(&app, DASHBOARD), Some(true));
        menu(&mut app, DASHBOARD, &json!({"show": false})).unwrap().unwrap();
        assert_eq!(checked(&app, DASHBOARD), Some(false));
        let all = procedures(&app, "", false);
        assert!(all.len() > 300);
        let some = procedures(&app, "magic wand", false);
        assert!(some.iter().any(|r| r.0 == "select.magicWand") && some.len() < all.len());
        let plug = procedures(&app, "", true);
        assert!(plug.iter().all(|r| !r.2.is_empty() || r.0.starts_with("plugin.")));
        assert!(handles(PROCEDURE_BROWSER) && !handles("view.zoomIn"));
    }

    #[test]
    fn dashboard_samples_and_logs_csv() {
        let mut app = app();
        assert!(sample(&mut app, 10.0));
        assert!(!sample(&mut app, 10.5), "inside the interval");
        assert!(sample(&mut app, 11.5));
        assert_eq!(app.ui.extra.samples.len(), 2);
        assert!(app.ui.extra.samples[1].documents > 0);
        let csv = log_csv(&app.ui.extra.samples);
        assert_eq!(csv.lines().count(), 3);
        assert!(csv.starts_with("t_s,rss_bytes"));
        assert!(!low_memory(&Sample { swap_total: 100, swap_used: 10, ..Default::default() }));
        assert!(low_memory(&Sample { swap_total: 100, swap_used: 95, ..Default::default() }));
    }

    #[test]
    fn devices_remember_their_tool_and_colours() {
        let mut app = app();
        app.ui.tool = Tool::Brush;
        app.run("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
        device_switch(&mut app, "Mouse");
        // The pen comes in: it starts from the mouse's state, then changes tool and colour.
        device_switch(&mut app, "Pen");
        crate::tool_memory::select(&mut app, Tool::Eraser);
        app.run("tools.setColors", json!({"foreground": "#00ff00"})).unwrap();
        // Back to the mouse: its brush and red foreground return.
        device_switch(&mut app, "Mouse");
        assert_eq!(app.ui.tool, Tool::Brush);
        assert!((app.session.tools.foreground[0] - 1.0).abs() < 1e-3 && app.session.tools.foreground[1] < 1e-3);
        // And the pen again: eraser and green.
        device_switch(&mut app, "Pen");
        assert_eq!(app.ui.tool, Tool::Eraser);
        assert!(app.session.tools.foreground[1] > 0.99);
        let r = save_devices(&mut app).unwrap();
        assert_eq!(r["devices"], 2);
        assert!(app.session.prefs().dialogs.contains_key("deviceStatus"));
    }

    #[test]
    fn selection_image_marks_the_selected_pixels() {
        let mut app = app();
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 16})).unwrap();
        let doc = app.session.active().unwrap().doc.clone();
        let img = selection_image(&doc, 16);
        assert_eq!(img.size, [16, 16]);
        assert_eq!(img.pixels[0], Color32::WHITE);
        assert_eq!(img.pixels[15], Color32::BLACK);
    }

    #[test]
    fn metadata_dialog_opens_with_the_document_fields() {
        let mut app = app();
        app.run("image.metadata.edit", json!({"title": "Hello"})).unwrap();
        let ctx = egui::Context::default();
        crate::menus::invoke(&mut app, &ctx, METADATA_EDIT, json!({})).unwrap();
        let (tab, fields) = app.ui.extra.metadata.clone().expect("dialog open");
        assert_eq!(tab, "Description");
        assert_eq!(fields["title"], "Hello");
        crate::menus::invoke(&mut app, &ctx, METADATA_VIEW, json!({})).unwrap();
        assert!(app.ui.extra.metadata_viewer);
    }
}
