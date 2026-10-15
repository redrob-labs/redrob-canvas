//! The Redrob Canvas shell: one window, a thin top bar, and the three upstream editors hosted as
//! switchable modes. Editors are created lazily on their first switch and then kept alive, so
//! switching modes preserves their open documents.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use designcraft_ui_egui::DesignApp;
use photocraft_ui_egui::PhotocraftApp;
use serde_json::{Value, json};
use vectorcraft_ui_egui::VectorcraftApp;

use crate::ai::bridge::{Capabilities, LayoutBridge, ModeBridge, PhotoBridge, VectorBridge};
use crate::ai::jobs::{JobOutput, JobResult, ToolKind};
use crate::ai::tools::{self, ToolReply, ToolRequest};
use crate::ai::{self, Action, AiPanel};
use crate::{handoff, services};

/// Height of the shell's own top bar, in points.
pub const TOP_BAR_HEIGHT: f32 = 28.0;

/// Which editor the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Mode {
    #[default]
    Photo,
    Vector,
    Layout,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Photo, Mode::Vector, Mode::Layout];

    /// The segmented button's label.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Photo => "Photo",
            Mode::Vector => "Vector",
            Mode::Layout => "Layout",
        }
    }

    /// Ctrl+1 / Ctrl+2 / Ctrl+3.
    pub fn shortcut(self) -> egui::KeyboardShortcut {
        let key = match self {
            Mode::Photo => egui::Key::Num1,
            Mode::Vector => egui::Key::Num2,
            Mode::Layout => egui::Key::Num3,
        };
        egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key)
    }
}

/// The window title the editors would set (`PhotoCraft`, `name.psd @ 100% - PhotoCraft`) read
/// back as this app's: the product suffix becomes "Redrob Canvas".
pub fn shell_title(current: &str) -> String {
    for upstream in ["PhotoCraft", "VectorCraft", "DesignCraft"] {
        if let Some(prefix) = current.strip_suffix(upstream) {
            return format!("{prefix}{}", services::APP_NAME);
        }
    }
    if current.is_empty() {
        return services::APP_NAME.to_string();
    }
    if current.ends_with(services::APP_NAME) {
        return current.to_string();
    }
    format!("{current} - {}", services::APP_NAME)
}

/// The editors send their own window title (`PhotocraftApp` on every document change); the
/// window reports it back the next frame, and the shell renames it once. Nothing is sent while
/// the title already is the shell's.
fn sync_window_title(ctx: &egui::Context) {
    let current = ctx.input(|i| i.viewport().title.clone());
    if let Some(current) = current {
        let want = shell_title(&current);
        if want != current {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(want));
        }
    }
}

/// The fonts an editor installed, by name: which font files and which families they form. Cheap
/// to compare every frame, unlike `FontDefinitions` itself (that compares the font bytes).
type FontSignature = (BTreeSet<String>, BTreeMap<String, Vec<String>>);

fn font_signature(defs: &egui::FontDefinitions) -> FontSignature {
    let names = defs.font_data.keys().cloned().collect();
    let families = defs
        .families
        .iter()
        .map(|(family, stack)| (format!("{family:?}"), stack.clone()))
        .collect();
    (names, families)
}

/// The union of every editor's font definitions. Every editor's named families are present, so
/// whichever editor draws finds its fonts; the generic families (`Proportional`, `Monospace`)
/// and any name two editors share come from `active`'s own definitions.
fn merge_fonts(all: &HashMap<Mode, egui::FontDefinitions>, active: Mode) -> egui::FontDefinitions {
    let mut merged = egui::FontDefinitions::default();
    merged.font_data.clear();
    merged.families.clear();
    let order = all
        .keys()
        .copied()
        .filter(|m| *m != active)
        .chain(all.contains_key(&active).then_some(active));
    for mode in order {
        if let Some(defs) = all.get(&mode) {
            for (name, data) in &defs.font_data {
                merged
                    .font_data
                    .entry(name.clone())
                    .or_insert_with(|| data.clone());
            }
            for (family, stack) in &defs.families {
                merged.families.insert(family.clone(), stack.clone());
            }
        }
    }
    merged
}

/// An editor's egui style, saved while it is active and restored when it is active again: each
/// editor applies its theme to the shared context once, on its first frame.
#[derive(Clone)]
struct StyleSnapshot {
    global: Arc<egui::Style>,
    dark: Arc<egui::Style>,
    light: Arc<egui::Style>,
    theme: egui::ThemePreference,
}

impl StyleSnapshot {
    fn take(ctx: &egui::Context) -> Self {
        Self {
            global: ctx.global_style(),
            dark: ctx.style_of(egui::Theme::Dark),
            light: ctx.style_of(egui::Theme::Light),
            theme: ctx.options(|o| o.theme_preference),
        }
    }

    fn restore(&self, ctx: &egui::Context) {
        ctx.set_global_style(self.global.clone());
        ctx.set_style_of(egui::Theme::Dark, self.dark.clone());
        ctx.set_style_of(egui::Theme::Light, self.light.clone());
        ctx.set_theme(self.theme);
    }
}

/// The eframe app: the shell plus its (lazily created) editors.
pub struct ShellApp {
    mode: Mode,
    /// A switch asked for (button, shortcut, `--mode`) and applied at the next frame's start,
    /// once the shared fonts hold every editor's families.
    pending: Option<Mode>,
    /// Each editor's own font definitions, read back after it installed them.
    fonts: HashMap<Mode, egui::FontDefinitions>,
    /// The signature of the merged definitions the shell installed last; `None` until an editor
    /// installed fonts of its own.
    merged_fonts: Option<FontSignature>,
    /// Each editor's egui style, saved every frame it is active.
    styles: HashMap<Mode, StyleSnapshot>,
    photo: Option<PhotocraftApp>,
    vector: Option<VectorcraftApp>,
    layout: Option<DesignApp>,
    /// The window's wgpu state, handed to the Photo editor's GPU canvas when it is created.
    wgpu: Option<eframe::egui_wgpu::RenderState>,
    /// Modes whose editor has been created, in creation order (for tests and the status line).
    created: Vec<Mode>,
    /// The AI dock, menu, engine, job queue and agent.
    pub ai: AiPanel,
    /// What the active mode's bridge could do when this frame's logic ran.
    caps: Capabilities,
}

impl Default for ShellApp {
    fn default() -> Self {
        Self::new(None)
    }
}

impl ShellApp {
    pub fn new(wgpu: Option<eframe::egui_wgpu::RenderState>) -> Self {
        // The engine installs under the config directory; a run without preferences has no
        // engine (the Tools tab says so).
        let ai = AiPanel::new(services::config_dir().filter(|_| services::prefs_enabled()));
        Self {
            mode: Mode::default(),
            pending: None,
            fonts: HashMap::new(),
            merged_fonts: None,
            styles: HashMap::new(),
            photo: None,
            vector: None,
            layout: None,
            wgpu,
            created: Vec::new(),
            ai,
            caps: Capabilities::none(Mode::default()),
        }
    }

    /// What the active mode's bridge could do at the start of this frame.
    pub fn capabilities(&self) -> Capabilities {
        self.caps
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Modes whose editor exists, in creation order.
    pub fn created(&self) -> &[Mode] {
        &self.created
    }

    /// Ask for `mode`: the switch is applied at the start of the next frame ([`Self::apply_pending`]).
    /// The editor is created on its first visit and kept afterwards.
    pub fn set_mode(&mut self, mode: Mode) {
        self.pending = Some(mode);
    }

    /// The mode asked for and not applied yet.
    pub fn pending_mode(&self) -> Option<Mode> {
        self.pending
    }

    /// Apply a requested switch. Not while an editor's own font definitions are the live ones
    /// (another editor would draw with families they lack and egui would panic): the shell
    /// installs the merged set first ([`Self::sync_fonts`]) and the switch waits one frame.
    pub fn apply_pending(&mut self, ctx: &egui::Context) {
        let Some(next) = self.pending else { return };
        if next == self.mode {
            self.pending = None;
            return;
        }
        if !self.fonts_settled(ctx) {
            return;
        }
        self.pending = None;
        self.styles.insert(self.mode, StyleSnapshot::take(ctx));
        self.mode = next;
        if let Some(style) = self.styles.get(&next) {
            style.restore(ctx);
        }
        if self.fonts.contains_key(&next) {
            let merged = merge_fonts(&self.fonts, next);
            self.merged_fonts = Some(font_signature(&merged));
            ctx.set_fonts(merged);
        }
    }

    /// Whether the live font definitions are egui's defaults or the shell's merged set.
    fn fonts_settled(&self, ctx: &egui::Context) -> bool {
        match &self.merged_fonts {
            None => self.fonts.is_empty(),
            Some(merged) => ctx.fonts(|f| font_signature(f.definitions()) == *merged),
        }
    }

    /// Each editor installs its own fonts with `Context::set_fonts`, replacing everyone else's.
    /// When the live definitions are not the shell's merged set, the active editor just did
    /// that: remember its set and install the union of all of them.
    fn sync_fonts(&mut self, ctx: &egui::Context) {
        let live = ctx.fonts(|f| font_signature(f.definitions()));
        if self.merged_fonts.as_ref() == Some(&live) {
            return;
        }
        if self.merged_fonts.is_none()
            && self.fonts.is_empty()
            && live == font_signature(&egui::FontDefinitions::default())
        {
            // Still egui's defaults: no editor has installed anything yet.
            return;
        }
        let defs = ctx.fonts(|f| f.definitions().clone());
        self.fonts.insert(self.mode, defs);
        let merged = merge_fonts(&self.fonts, self.mode);
        self.merged_fonts = Some(font_signature(&merged));
        ctx.set_fonts(merged);
    }

    /// The shell's own top bar: brand, the mode buttons, the AI menu, and the right slot.
    pub fn top_bar(&mut self, ui: &mut egui::Ui) {
        let caps = self.caps;
        let visuals = ui.visuals().clone();
        let frame = egui::Frame::NONE
            .fill(visuals.panel_fill)
            .stroke(egui::Stroke::new(
                1.0,
                visuals.widgets.noninteractive.bg_stroke.color,
            ));
        egui::Panel::top("redrob_canvas_top_bar")
            .exact_size(TOP_BAR_HEIGHT)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(services::APP_NAME).strong());
                    ui.add_space(12.0);
                    ui.separator();
                    let mut next = self.mode;
                    for mode in Mode::ALL {
                        let selected = ui
                            .selectable_label(self.mode == mode, mode.label())
                            .on_hover_text(format!(
                                "{}  ({})",
                                mode.label(),
                                ui.ctx().format_shortcut(&mode.shortcut())
                            ));
                        if selected.clicked() {
                            next = mode;
                        }
                    }
                    if next != self.mode {
                        self.set_mode(next);
                    }
                    ui.separator();
                    ai::panel::menu(ui, &mut self.ai, &caps);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        self.right_slot(ui);
                    });
                });
            });
    }

    /// The right side of the top bar: the AI dock toggle.
    pub fn right_slot(&mut self, ui: &mut egui::Ui) {
        ai::panel::toggle(ui, &mut self.ai);
    }

    /// The AI dock, drawn before the editor area so the editor gets what is left. Returns the
    /// actions its widgets asked for.
    fn ai_dock(&mut self, ui: &mut egui::Ui) -> Vec<Action> {
        let mut actions = std::mem::take(&mut self.ai.queued);
        if !self.ai.open {
            return actions;
        }
        let caps = self.caps;
        let visuals = ui.visuals().clone();
        let frame = egui::Frame::NONE
            .fill(visuals.panel_fill)
            .inner_margin(8.0)
            .stroke(egui::Stroke::new(
                1.0,
                visuals.widgets.noninteractive.bg_stroke.color,
            ));
        let drawn = egui::Panel::right("redrob_canvas_ai_dock")
            .default_size(ai::panel::DEFAULT_WIDTH)
            .size_range(ai::panel::MIN_WIDTH..=ai::panel::MAX_WIDTH)
            .resizable(true)
            .frame(frame)
            .show(ui, |ui| ai::panel::dock(ui, &mut self.ai, &caps));
        actions.extend(drawn.inner);
        actions
    }

    /// The active mode's document, through its bridge. Creates the mode's editor when it does not
    /// exist yet (the frame would anyway).
    pub fn with_bridge<R>(&mut self, f: impl FnOnce(&mut dyn ModeBridge) -> R) -> R {
        match self.mode {
            Mode::Photo => {
                let app = self.ensure_photo();
                f(&mut PhotoBridge::new(&mut app.session))
            }
            Mode::Vector => {
                let app = self.ensure_vector();
                f(&mut VectorBridge::new(app))
            }
            Mode::Layout => {
                let app = self.ensure_layout();
                f(&mut LayoutBridge::new(app))
            }
        }
    }

    /// The Vector editor (created on demand): where a trace hand-off lands.
    pub fn vector_app(&mut self) -> &mut VectorcraftApp {
        self.ensure_vector()
    }

    /// Everything the AI layer does between frames: finished jobs, the agent's tool calls, agent
    /// events, the engine log, and the capabilities and info the next frame draws from.
    fn service_ai(&mut self, ctx: &egui::Context) {
        while let Some(result) = self.ai.poll_job() {
            self.apply_ai_result(result);
        }
        for request in self.ai.take_tool_requests() {
            self.handle_tool_request(request);
        }
        self.ai.poll_agent();
        self.ai.drain_engine_log();
        let (caps, size) = self.with_bridge(|b| (b.capabilities(), b.document_size()));
        self.caps = caps;
        self.ai.refresh_info(self.mode, &caps, size);
        if self.ai.wants_repaint() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn run_ai_action(&mut self, action: Action) {
        match action {
            Action::Run(kind) => {
                if let Err(e) = self.start_tool(kind, None, None) {
                    self.ai.status = Some((format!("{}: {e}", kind.label()), true));
                    self.ai.push_log(format!("{}: {e}", kind.label()));
                }
            }
            Action::Install => self.ai.install(),
            Action::Start => self.ai.start(),
            Action::Stop => self.ai.stop(),
            Action::SetModel => self.ai.set_model(),
            Action::SwitchMode(mode) => self.set_mode(mode),
            Action::AgentSend(text) => self.ai.agent_send(text),
            Action::AgentReset => self.ai.agent_reset(),
            Action::CancelJob => {
                if let Some(reply) = self.ai.jobs.cancel() {
                    let _ = reply.send(Err("cancelled".into()));
                }
                self.ai.status = Some((
                    "Cancelled; the engine finishes its request in the background".into(),
                    false,
                ));
            }
        }
    }

    /// Gathers `kind`'s inputs from the active mode and starts it. `args` are the agent's tool
    /// arguments (they override the card's settings); `reply` its pending call.
    fn start_tool(
        &mut self,
        kind: ToolKind,
        reply: Option<ToolReply>,
        args: Option<&Value>,
    ) -> Result<(), String> {
        if let Some(reason) = kind.unavailable_reason(&self.caps) {
            return Err(reason.to_owned());
        }
        let settings = self.ai.settings.clone();
        if let Some(plugin) = ai::plugin_for(kind, &settings)
            && !self
                .ai
                .engine()
                .and_then(|e| e.config())
                .is_some_and(|c| c.has_plugin(plugin))
        {
            return Err(format!(
                "start the AI engine with the {plugin} plugin enabled (Plugins, above)"
            ));
        }
        let input = self.with_bridge(|bridge| ai::gather_input(bridge, kind, &settings, args))?;
        self.ai.start_job(kind, input, reply)
    }

    fn apply_ai_result(&mut self, result: JobResult) {
        let kind = result.kind;
        let outcome = match result.output {
            Err(e) => Err(e),
            Ok(output) => self.apply_ai_output(kind, output),
        };
        let seconds = result.elapsed.as_secs_f32();
        match &outcome {
            Ok(_) => {
                let line = format!("{} done in {seconds:.1} s", kind.label());
                self.ai.status = Some((line.clone(), false));
                self.ai.push_log(line);
            }
            Err(e) => {
                let line = format!("{}: {e}", kind.label());
                self.ai.status = Some((line.clone(), true));
                self.ai.push_log(line);
            }
        }
        if let Some(reply) = result.reply {
            let _ = reply.send(outcome);
        }
    }

    /// Puts a job's output into the active mode's document.
    fn apply_ai_output(&mut self, kind: ToolKind, output: JobOutput) -> Result<Value, String> {
        match output {
            JobOutput::Layer {
                name,
                image,
                offset,
            } => {
                let (w, h) = (image.width, image.height);
                self.with_bridge(|bridge| {
                    // An upscale is larger than the canvas: grow the canvas first where the
                    // mode can, so the layer is not cropped.
                    if kind == ToolKind::Upscale
                        && bridge.supports_expand()
                        && let Some((dw, dh)) = bridge.document_size()
                        && (w > dw || h > dh)
                    {
                        bridge
                            .expand_canvas(redrob_ai::Expand {
                                left: 0,
                                top: 0,
                                right: w.saturating_sub(dw),
                                bottom: h.saturating_sub(dh),
                            })
                            .map_err(|e| e.to_string())?;
                    }
                    bridge
                        .place_layer(&name, image, offset)
                        .map_err(|e| e.to_string())
                })?;
                Ok(json!({ "ok": true, "layer": name, "width": w, "height": h }))
            }
            JobOutput::Expanded {
                name,
                image,
                expand,
            } => {
                let (w, h) = (image.width, image.height);
                self.with_bridge(|bridge| {
                    bridge.expand_canvas(expand).map_err(|e| e.to_string())?;
                    bridge
                        .place_layer(&name, image, (0, 0))
                        .map_err(|e| e.to_string())
                })?;
                Ok(json!({ "ok": true, "layer": name, "width": w, "height": h }))
            }
            JobOutput::Selection(mask) => {
                let coverage = mask.coverage();
                self.with_bridge(|bridge| bridge.set_selection(mask).map_err(|e| e.to_string()))?;
                Ok(json!({ "ok": true, "selected_pixels": coverage }))
            }
            JobOutput::Trace {
                preset,
                result,
                size,
            } => {
                let paths = handoff::deliver_trace(self, &result, size)?;
                Ok(json!({ "ok": true, "paths": paths, "preset": preset, "mode": "vector" }))
            }
        }
    }

    /// One of the agent's tool calls, answered through its reply channel (long tools answer when
    /// their job result is applied).
    fn handle_tool_request(&mut self, request: ToolRequest) {
        let ToolRequest {
            name,
            arguments,
            reply,
        } = request;
        if name == tools::SWITCH_MODE {
            let answer = match tools::parse_mode(arguments["mode"].as_str().unwrap_or_default()) {
                Some(mode) => {
                    self.set_mode(mode);
                    Ok(json!({ "ok": true, "mode": mode.label().to_lowercase() }))
                }
                None => Err("mode must be photo, vector or layout".to_owned()),
            };
            let _ = reply.send(answer);
            return;
        }
        let Some(kind) = ToolKind::from_tool_name(&name) else {
            let _ = reply.send(Err(format!("unknown tool {name}")));
            return;
        };
        self.ai.open = true;
        if let Err(e) = self.start_tool(kind, Some(reply.clone()), Some(&arguments)) {
            self.ai.push_log(format!("agent {}: {e}", kind.label()));
            let _ = reply.send(Err(e));
        }
    }

    /// Ctrl+1/2/3 switch modes. Consumed before the active editor sees the frame's input.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        for mode in Mode::ALL {
            if ctx.input_mut(|i| i.consume_shortcut(&mode.shortcut())) {
                self.set_mode(mode);
            }
        }
    }

    fn ensure_photo(&mut self) -> &mut PhotocraftApp {
        if self.photo.is_none() {
            self.created.push(Mode::Photo);
        }
        let wgpu = self.wgpu.clone();
        self.photo.get_or_insert_with(|| Self::build_photo(wgpu))
    }

    fn build_photo(wgpu: Option<eframe::egui_wgpu::RenderState>) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(
            photocraft_engine::Session::new(),
            services::photo::services(),
        );
        // Long commands and file opens run as background jobs with progress and Cancel.
        app.background_jobs = std::env::var_os("PHOTOCRAFT_INLINE_JOBS").is_none();
        if let Some(rs) = wgpu
            && rs.adapter.get_info().device_type != eframe::wgpu::DeviceType::Cpu
            && std::env::var_os("PHOTOCRAFT_CPU_CANVAS").is_none()
        {
            app.set_wgpu(rs);
        }
        app
    }

    fn ensure_vector(&mut self) -> &mut VectorcraftApp {
        if self.vector.is_none() {
            self.created.push(Mode::Vector);
        }
        self.vector.get_or_insert_with(Self::build_vector)
    }

    fn build_vector() -> VectorcraftApp {
        services::vector::install_fonts();
        let mut app = VectorcraftApp::new(
            vectorcraft_engine::Session::new(),
            services::vector::services(),
        );
        services::vector::load_prefs(&mut app);
        services::vector::attach_dirs(&mut app);
        app
    }

    fn ensure_layout(&mut self) -> &mut DesignApp {
        if self.layout.is_none() {
            self.created.push(Mode::Layout);
        }
        self.layout.get_or_insert_with(Self::build_layout)
    }

    fn build_layout() -> DesignApp {
        let mut session = designcraft_engine::Session::new();
        // Crash recovery: reopen what a previous run left unsaved, then keep it current.
        session.recovery_dir = services::layout::recovery_dir();
        let recovered = session
            .execute("file.recovery.open", &serde_json::json!({}))
            .ok();
        let mut app = DesignApp::new(session, services::layout::services());
        if let Some(n) = recovered
            .as_ref()
            .and_then(|r| r["opened"].as_array())
            .map(Vec::len)
            .filter(|n| *n > 0)
        {
            app.status(format!(
                "Recovered {n} unsaved document{} from the last session.",
                if n == 1 { "" } else { "s" }
            ));
        }
        services::layout::load_prefs(&mut app);
        app
    }

    /// The editor area below the top bar: the active editor draws its own panels inside it.
    fn editor_area(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let rect = ui.available_rect_before_wrap();
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        match self.mode() {
            Mode::Photo => eframe::App::ui(self.ensure_photo(), &mut child, frame),
            Mode::Vector => self.ensure_vector().ui(&mut child),
            Mode::Layout => self.ensure_layout().ui(&mut child),
        }
    }
}

impl eframe::App for ShellApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.handle_shortcuts(ctx);
        self.sync_fonts(ctx);
        self.apply_pending(ctx);
        if self.pending.is_some() {
            // The switch waits for the merged fonts: keep the frames coming.
            ctx.request_repaint();
        }
        // Only the active editor runs its frame logic (and so sees this frame's input).
        match self.mode {
            Mode::Photo => eframe::App::logic(self.ensure_photo(), ctx, frame),
            Mode::Vector => {
                let app = self.ensure_vector();
                app.logic(ctx);
                if app.take_prefs_changed() {
                    services::vector::save_prefs(app);
                }
                if app.ui.status == "quit" {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Mode::Layout => self.ensure_layout().logic(ctx),
        }
        self.service_ai(ctx);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        match self.mode {
            Mode::Photo => {
                if let Some(app) = self.photo.as_mut() {
                    eframe::App::raw_input_hook(app, ctx, raw);
                }
            }
            Mode::Vector => {
                if let Some(app) = self.vector.as_mut() {
                    app.raw_input_hook(raw);
                }
            }
            Mode::Layout => {
                if let Some(app) = self.layout.as_mut() {
                    app.raw_input_hook(raw);
                }
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        sync_window_title(ui.ctx());
        self.top_bar(ui);
        let actions = self.ai_dock(ui);
        for action in actions {
            self.run_ai_action(action);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.editor_area(ui, frame));
        self.styles.insert(self.mode, StyleSnapshot::take(ui.ctx()));
    }

    fn on_exit(&mut self) {
        log::info!(
            "closing in {} mode (pending {:?}); editors created: {:?}",
            self.mode().label(),
            self.pending_mode(),
            self.created()
        );
        if let Some(app) = self.photo.as_mut() {
            eframe::App::on_exit(app);
        }
        if let Some(app) = self.vector.as_ref() {
            services::vector::save_prefs(app);
        }
        if let Some(app) = self.layout.as_mut() {
            if let Err(e) = services::layout::preserve_documents_on_exit(app) {
                log::error!("Couldn't preserve layout documents on exit: {e}");
            }
            services::layout::save_prefs(app);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_switch_and_each_has_a_distinct_shortcut() {
        let ctx = egui::Context::default();
        let mut shell = ShellApp::new(None);
        assert_eq!(shell.mode(), Mode::Photo);
        for mode in [Mode::Vector, Mode::Layout, Mode::Photo] {
            shell.set_mode(mode);
            assert_eq!(shell.pending_mode(), Some(mode));
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                shell.apply_pending(ui.ctx())
            });
            out.textures_delta.clear();
            assert_eq!((shell.mode(), shell.pending_mode()), (mode, None));
        }
        let keys: std::collections::BTreeSet<_> = Mode::ALL
            .iter()
            .map(|m| format!("{:?}", m.shortcut().logical_key))
            .collect();
        assert_eq!(keys.len(), 3);
        assert!(
            Mode::ALL
                .iter()
                .all(|m| m.shortcut().modifiers == egui::Modifiers::COMMAND)
        );
    }

    #[test]
    fn ctrl_digits_switch_modes_before_the_editor_sees_them() {
        let mut shell = ShellApp::new(None);
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::Key {
            key: egui::Key::Num2,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        let mut out = ctx.run_ui(raw, |ui| {
            shell.handle_shortcuts(ui.ctx());
            // The shortcut was consumed: the active editor cannot see it any more.
            assert!(
                !ui.ctx()
                    .input_mut(|i| i.consume_shortcut(&Mode::Vector.shortcut()))
            );
        });
        out.textures_delta.clear();
        assert_eq!(shell.pending_mode(), Some(Mode::Vector));
        assert!(
            shell.created().is_empty(),
            "switching alone creates no editor; the frame does"
        );
    }

    #[test]
    fn a_switch_waits_until_the_merged_fonts_are_live() {
        let ctx = egui::Context::default();
        let mut shell = ShellApp::new(None);
        let frame = |shell: &mut ShellApp, f: &dyn Fn(&egui::Context)| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                f(ui.ctx());
                shell.sync_fonts(ui.ctx());
                shell.apply_pending(ui.ctx());
            });
            out.textures_delta.clear();
        };
        let named = |name: &str| {
            let mut defs = egui::FontDefinitions::default();
            defs.families
                .insert(egui::FontFamily::Name(name.into()), vec!["Hack".into()]);
            defs
        };
        // Frame 1: the Photo editor installs fonts of its own, with a named family.
        frame(&mut shell, &|ctx| ctx.set_fonts(named("photo-ui")));
        assert!(
            shell.fonts.is_empty(),
            "set_fonts applies at the next frame"
        );
        // Frame 2: the shell records them; one editor's set is the merged set, so a switch asked
        // for now goes through at once.
        shell.set_mode(Mode::Vector);
        frame(&mut shell, &|_| {});
        assert!(shell.fonts.contains_key(&Mode::Photo));
        assert_eq!((shell.mode(), shell.pending_mode()), (Mode::Vector, None));
        // Frame 3: the Vector editor installs its own set, dropping Photo's family.
        frame(&mut shell, &|ctx| ctx.set_fonts(named("vector-ui")));
        // Frame 4: Vector's set is live and is not the merged one: a switch back must wait...
        shell.set_mode(Mode::Photo);
        frame(&mut shell, &|_| {});
        assert!(shell.fonts.contains_key(&Mode::Vector));
        assert_eq!(
            (shell.mode(), shell.pending_mode()),
            (Mode::Vector, Some(Mode::Photo))
        );
        // Frame 5: ...until the union is live: both named families present.
        frame(&mut shell, &|_| {});
        let live = ctx.fonts(|f| f.definitions().clone());
        assert!(
            live.families
                .contains_key(&egui::FontFamily::Name("photo-ui".into()))
        );
        assert!(
            live.families
                .contains_key(&egui::FontFamily::Name("vector-ui".into()))
        );
        assert_eq!((shell.mode(), shell.pending_mode()), (Mode::Photo, None));
        assert!(shell.fonts_settled(&ctx));
    }

    #[test]
    fn merged_fonts_prefer_the_active_editors_generic_families() {
        let mut a = egui::FontDefinitions::default();
        a.families
            .insert(egui::FontFamily::Proportional, vec!["A".into()]);
        a.families
            .insert(egui::FontFamily::Name("shared".into()), vec!["A".into()]);
        let mut b = egui::FontDefinitions::default();
        b.families
            .insert(egui::FontFamily::Proportional, vec!["B".into()]);
        b.families
            .insert(egui::FontFamily::Name("shared".into()), vec!["B".into()]);
        b.families
            .insert(egui::FontFamily::Name("only-b".into()), vec!["B".into()]);
        let all = HashMap::from([(Mode::Photo, a), (Mode::Vector, b)]);
        let merged = merge_fonts(&all, Mode::Photo);
        assert_eq!(
            merged.families[&egui::FontFamily::Proportional],
            vec!["A".to_string()]
        );
        assert_eq!(
            merged.families[&egui::FontFamily::Name("shared".into())],
            vec!["A".to_string()]
        );
        assert!(
            merged
                .families
                .contains_key(&egui::FontFamily::Name("only-b".into()))
        );
        let merged = merge_fonts(&all, Mode::Vector);
        assert_eq!(
            merged.families[&egui::FontFamily::Proportional],
            vec!["B".to_string()]
        );
        assert_eq!(
            merged.font_data.len(),
            egui::FontDefinitions::default().font_data.len()
        );
    }

    #[test]
    fn editor_titles_are_rewritten_to_the_shells() {
        assert_eq!(shell_title("PhotoCraft"), "Redrob Canvas");
        assert_eq!(
            shell_title("Untitled-1 @ 100% (RGB/8) - PhotoCraft"),
            "Untitled-1 @ 100% (RGB/8) - Redrob Canvas"
        );
        assert_eq!(shell_title("VectorCraft"), "Redrob Canvas");
        assert_eq!(shell_title("Redrob Canvas"), "Redrob Canvas");
        assert_eq!(
            shell_title("poster.designcraft - Redrob Canvas"),
            "poster.designcraft - Redrob Canvas"
        );
        assert_eq!(shell_title(""), "Redrob Canvas");
        assert_eq!(
            shell_title("Something else"),
            "Something else - Redrob Canvas"
        );
    }

    #[test]
    fn the_top_bar_switches_modes_by_click() {
        use egui_kittest::kittest::Queryable;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 100.0))
            .build_ui_state(
                |ui, shell: &mut ShellApp| shell.top_bar(ui),
                ShellApp::new(None),
            );
        harness.run();
        harness.get_by_label("Redrob Canvas");
        harness.get_by_label("Layout").click();
        harness.run();
        assert_eq!(harness.state().pending_mode(), Some(Mode::Layout));
        let ctx = harness.ctx.clone();
        harness.state_mut().apply_pending(&ctx);
        assert_eq!(harness.state().mode(), Mode::Layout);
        harness.get_by_label("Vector").click();
        harness.run();
        assert_eq!(harness.state().pending_mode(), Some(Mode::Vector));
    }
}
