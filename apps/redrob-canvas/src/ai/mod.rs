// SPDX-License-Identifier: MIT OR Apache-2.0

//! The AI layer in the shell: the right dock's state ([`AiPanel`]), the tokio runtime the
//! asynchronous `redrob-ai` crate runs on, the local IOPaint [`Engine`], the one-job queue, the
//! engine log and the agent chat. The shell owns one `AiPanel`; `panel` draws it, `bridge` reads
//! and writes the editors' documents, `jobs` runs the work, `tools` serves the agent.

pub mod bridge;
pub mod jobs;
pub mod panel;
pub mod tools;

use std::sync::{
    Arc, mpsc,
    atomic::{AtomicBool, Ordering},
};

use redrob_ai::{
    Expand,
    agent::{AgentEvent, AgentHandle, AgentOptions},
    console::{ConsoleImages, ImageModel, Operation},
    iopaint::{
        Device, Engine, EnginePaths, EngineState, InpaintOptions, Plugin, TransportMode, models,
    },
};
use serde_json::Value;
use tokio::sync::broadcast;

use self::jobs::{ConsoleJob, JobInput, JobQueue, JobResult, ModelChoice, ToolKind};
use self::tools::{ModeInfo, ShellTools, ToolRequest};
use crate::shell::Mode;

/// The dock's two tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Tools,
    Agent,
}

/// Lines of engine / job log the panel keeps.
pub const MAX_LOG: usize = 300;

/// What the panel and the menu ask the shell to do. Collected while drawing, applied after.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Run(ToolKind),
    Install,
    Start,
    Stop,
    /// The model, subject model or transport setting changed; the running engine follows.
    SetModel,
    AgentSend(String),
    AgentReset,
    CancelJob,
    /// Ask the Redrob Console again which image models it serves (after signing in).
    RefreshConsole,
}

/// Where Erase, Generative Fill and Expand run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backend {
    /// The local IOPaint engine.
    #[default]
    Local,
    /// The Redrob Console image models, with the user's console sign-in.
    Console,
}

/// The `x-redrob-app` label the console image requests carry.
pub const CONSOLE_APP: &str = "redrob-canvas";

/// The cards' parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSettings {
    pub model: String,
    pub device: Device,
    pub plugins: Vec<Plugin>,
    pub prompt: String,
    pub negative_prompt: String,
    /// Generative Fill's prompt (its own, so an erase model can stay selected above).
    pub fill_prompt: String,
    pub fill_negative_prompt: String,
    /// The diffusion model Generative Fill loads (one of `models::DIFFUSION_MODELS` or a
    /// downloaded model).
    pub fill_model: String,
    /// How pixels reach the engine: `file` (default, local engine) or `base64`.
    pub transport: TransportMode,
    /// The RemoveBG plugin's model for Select subject / Remove background (isnet by default).
    pub remove_bg_model: String,
    pub steps: u32,
    pub strength: f32,
    pub guidance: f32,
    pub seed: i64,
    pub expand: Expand,
    pub upscale: u32,
    /// Where Erase, Generative Fill and Expand run. Only `Console` while the console lists image
    /// models; the panel falls back to `Local` when it stops.
    pub backend: Backend,
    /// The console model each operation uses (empty = the operation's default).
    pub console_fill_model: String,
    pub console_expand_model: String,
    pub console_erase_model: String,
    pub console_generate_model: String,
    /// Generate's prompt.
    pub generate_prompt: String,
    /// `"GFPGAN"` or `"RestoreFormer"`.
    pub face_plugin: &'static str,
    /// `"RemoveBG"` or `"AnimeSeg"`.
    pub subject_plugin: &'static str,
    pub preset: String,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            model: models::DEFAULT_MODEL.to_owned(),
            device: Device::default(),
            plugins: Plugin::DEFAULT.to_vec(),
            prompt: String::new(),
            negative_prompt: String::new(),
            fill_prompt: String::new(),
            fill_negative_prompt: String::new(),
            fill_model: models::DEFAULT_FILL_MODEL.to_owned(),
            transport: TransportMode::default(),
            remove_bg_model: models::DEFAULT_REMOVE_BG_MODEL.to_owned(),
            steps: 25,
            strength: 1.0,
            guidance: 7.5,
            seed: -1,
            expand: Expand {
                left: 0,
                top: 0,
                right: 128,
                bottom: 0,
            },
            upscale: 2,
            backend: Backend::Local,
            console_fill_model: String::new(),
            console_expand_model: String::new(),
            console_erase_model: String::new(),
            console_generate_model: String::new(),
            generate_prompt: String::new(),
            face_plugin: "GFPGAN",
            subject_plugin: "RemoveBG",
            preset: redrob_ai::vectorize::preset_names()
                .first()
                .copied()
                .unwrap_or("default")
                .to_owned(),
        }
    }
}

impl ToolSettings {
    /// The diffusion options the Inpaint / Outpaint cards send; `prompt` overrides the card's.
    pub fn inpaint_options(&self, prompt: Option<&str>, negative: Option<&str>) -> InpaintOptions {
        let prompt = prompt.unwrap_or(&self.prompt).trim().to_owned();
        let negative = negative.unwrap_or(&self.negative_prompt).trim().to_owned();
        InpaintOptions {
            prompt: (!prompt.is_empty()).then_some(prompt),
            negative_prompt: (!negative.is_empty()).then_some(negative),
            sd_steps: Some(self.steps),
            sd_strength: Some(f64::from(self.strength)),
            sd_guidance_scale: Some(f64::from(self.guidance)),
            sd_seed: Some(self.seed),
            sd_keep_unmasked_area: Some(true),
            ..InpaintOptions::default()
        }
    }

    /// The options a Generative Fill sends: the fill card's prompt (overridden by `prompt`), the
    /// shared sampling settings, the result clipped to the selection.
    pub fn fill_options(&self, prompt: Option<&str>, negative: Option<&str>) -> InpaintOptions {
        let prompt = prompt.unwrap_or(&self.fill_prompt).trim().to_owned();
        let negative = negative
            .unwrap_or(&self.fill_negative_prompt)
            .trim()
            .to_owned();
        InpaintOptions {
            prompt: (!prompt.is_empty()).then_some(prompt),
            negative_prompt: (!negative.is_empty()).then_some(negative),
            ..self.inpaint_options(Some(""), Some(""))
        }
    }

    /// The console model `op` uses.
    pub fn console_model(&self, op: Operation) -> &str {
        match op {
            Operation::Fill => &self.console_fill_model,
            Operation::Expand => &self.console_expand_model,
            Operation::Erase => &self.console_erase_model,
            Operation::Generate => &self.console_generate_model,
        }
    }

    pub fn console_model_mut(&mut self, op: Operation) -> &mut String {
        match op {
            Operation::Fill => &mut self.console_fill_model,
            Operation::Expand => &mut self.console_expand_model,
            Operation::Erase => &mut self.console_erase_model,
            Operation::Generate => &mut self.console_generate_model,
        }
    }

    /// Whether `kind` runs on the Redrob Console with these settings.
    pub fn routes_to_console(&self, kind: ToolKind) -> bool {
        kind == ToolKind::Generate
            || (self.backend == Backend::Console && kind.console_operation().is_some())
    }

    /// The seed a console request sends (`-1` = random, so none).
    pub fn console_seed(&self) -> Option<i64> {
        (self.seed >= 0).then_some(self.seed)
    }

    /// The model, device and plugins Generative Fill holds.
    pub fn fill_model_choice(&self) -> ModelChoice {
        ModelChoice {
            model: self.fill_model.clone(),
            device: self.device,
            plugins: self.plugins.clone(),
        }
    }
}

/// Who said a chat line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speaker {
    User,
    Agent,
    Tool,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bubble {
    pub speaker: Speaker,
    pub text: String,
}

/// The Agent tab's state. The handle is created on the first Send, so the Redrob Code daemon is
/// only looked for when a prompt needs it.
#[derive(Default)]
pub struct AgentChat {
    pub handle: Option<AgentHandle>,
    pub transcript: Vec<Bubble>,
    pub input: String,
    /// The last error, shown above the input ("Redrob Code is not running" when the daemon is
    /// unavailable).
    pub error: Option<String>,
    pub busy: bool,
}

/// The message shown when the shared daemon cannot be reached.
pub const ENGINE_DOWN: &str = "Redrob Code is not running. Start it (or install it from redrob.ai) \
                               and send again.";

/// Whether an agent error means the shared Redrob Code daemon is unavailable (the texts are
/// `redrob_agent::Error::EngineUnavailable` / `EngineNotAuthenticated`).
pub fn is_engine_down(error: &str) -> bool {
    let e = error.to_ascii_lowercase();
    e.contains("redrob code") || e.contains("engine") || e.contains("connection refused")
}

/// What the agent is told about the window.
pub const SYSTEM_PROMPT: &str = "You are the Redrob Canvas agent. The window hosts three editors \
as modes: Photo (pixels, layers, selections), Vector (paths on artboards) and Layout (pages). Use \
snapshot_info first to learn the mode and document. Tools that need a selection or a pixel canvas \
work in Photo mode (generative_fill paints a prompt into the selection); remove_background, \
upscale, face_restore and vectorize work in every mode. \
vectorize switches to Vector mode with the traced paths. Say briefly what you did; do not invent \
results a tool did not return.";

/// Whether the console's image models have been asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Probe {
    #[default]
    NotStarted,
    Running,
    Done,
}

/// The Redrob Console backend: its client and the image models it listed. Asked for when the
/// dock opens (that is when the shared daemon is looked up), and again on Refresh, when the dock
/// is reopened while the console listed nothing, and after a job found the sign-in gone.
#[derive(Default)]
pub struct ConsoleState {
    client: Option<ConsoleImages>,
    pub models: Vec<ImageModel>,
    probe: Probe,
    rx: Option<mpsc::Receiver<Result<Vec<ImageModel>, String>>>,
    signed_out: Arc<AtomicBool>,
    was_open: bool,
}

impl ConsoleState {
    /// Whether the console backend is offered: a client and at least one image model.
    pub fn available(&self) -> bool {
        self.client.is_some() && !self.models.is_empty()
    }

    /// Whether the model list is being fetched.
    pub fn probing(&self) -> bool {
        self.probe == Probe::Running
    }

    /// The models `op` can use.
    pub fn models_for(&self, op: Operation) -> Vec<ImageModel> {
        op.models(&self.models)
    }
}

/// The state behind the AI dock.
pub struct AiPanel {
    pub open: bool,
    pub tab: Tab,
    runtime: Option<tokio::runtime::Runtime>,
    engine: Option<Engine>,
    engine_log: Option<broadcast::Receiver<String>>,
    pub settings: ToolSettings,
    /// The model the combo showed last frame, to notice a change.
    pub shown_model: String,
    /// The subject model and transport the engine row showed last frame, likewise.
    pub shown_remove_bg_model: String,
    pub shown_transport: TransportMode,
    pub jobs: JobQueue,
    pub log: Vec<String>,
    /// The last job's outcome, shown under the cards (`true` = error).
    pub status: Option<(String, bool)>,
    pub agent: AgentChat,
    pub console: ConsoleState,
    tools: Arc<ShellTools>,
    tool_rx: mpsc::Receiver<ToolRequest>,
    /// Actions the top bar / menu asked for, applied by the shell after drawing.
    pub queued: Vec<Action>,
}

impl Default for AiPanel {
    fn default() -> Self {
        Self::new(None)
    }
}

impl AiPanel {
    /// `data_dir` is where the engine installs (`<data_dir>/iopaint`); `None` runs without an
    /// engine (tests, no config directory): the cards then say so.
    pub fn new(data_dir: Option<std::path::PathBuf>) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("redrob-ai")
            .enable_all()
            .build()
            .map_err(|e| log::error!("no AI runtime: {e}"))
            .ok();
        let engine = data_dir.map(|dir| Engine::new(EnginePaths::new(dir)));
        let engine_log = engine.as_ref().map(Engine::subscribe_log);
        let (tools, tool_rx) = ShellTools::new();
        let settings = ToolSettings::default();
        Self {
            open: false,
            tab: Tab::Tools,
            runtime,
            engine,
            engine_log,
            shown_model: settings.model.clone(),
            shown_remove_bg_model: settings.remove_bg_model.clone(),
            shown_transport: settings.transport,
            settings,
            jobs: JobQueue::new(),
            log: Vec::new(),
            status: None,
            agent: AgentChat::default(),
            console: ConsoleState {
                client: ConsoleImages::new(CONSOLE_APP)
                    .map_err(|e| log::warn!("no Redrob Console image client: {e}"))
                    .ok(),
                ..ConsoleState::default()
            },
            tools: Arc::new(tools),
            tool_rx,
            queued: Vec::new(),
        }
    }

    pub fn engine(&self) -> Option<&Engine> {
        self.engine.as_ref()
    }

    pub fn engine_state(&self) -> EngineState {
        self.engine
            .as_ref()
            .map(Engine::state)
            .unwrap_or(EngineState::Missing)
    }

    pub fn engine_message(&self) -> String {
        match &self.engine {
            Some(engine) => engine.message(),
            None => "No data directory: the AI engine cannot be installed".to_owned(),
        }
    }

    /// Whether a card's Run should be disabled because something is running.
    pub fn is_busy(&self) -> bool {
        self.jobs.is_busy() || self.engine_state().is_busy()
    }

    /// Whether anything is in flight that needs frames to keep coming.
    pub fn wants_repaint(&self) -> bool {
        self.is_busy() || self.agent.busy
    }

    pub fn push_log(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
        if self.log.len() > MAX_LOG {
            let extra = self.log.len() - MAX_LOG;
            self.log.drain(..extra);
        }
    }

    /// Engine log lines since the last frame.
    pub fn drain_engine_log(&mut self) {
        let Some(rx) = self.engine_log.as_mut() else {
            return;
        };
        let mut lines = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(line) => lines.push(line),
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
        for line in lines {
            self.push_log(line);
        }
    }

    /// Spawns `fut` on the runtime; `false` (and a status line) when there is none.
    fn spawn(&mut self, fut: impl std::future::Future<Output = ()> + Send + 'static) -> bool {
        match &self.runtime {
            Some(rt) => {
                rt.spawn(fut);
                true
            }
            None => {
                self.status = Some(("the AI runtime did not start".into(), true));
                false
            }
        }
    }

    pub fn install(&mut self) {
        let Some(engine) = self.engine.clone() else {
            self.status = Some((self.engine_message(), true));
            return;
        };
        let device = self.settings.device;
        self.spawn(async move {
            if let Err(e) = engine.install(device).await {
                log::warn!("AI engine install: {e}");
            }
        });
    }

    pub fn start(&mut self) {
        let Some(engine) = self.engine.clone() else {
            self.status = Some((self.engine_message(), true));
            return;
        };
        let model = self.settings.model.clone();
        let device = self.settings.device;
        let plugins = self.settings.plugins.clone();
        engine.set_transport_mode(self.settings.transport);
        let remove_bg_model = self.settings.remove_bg_model.clone();
        self.spawn(async move {
            // Not running yet, so this only records the model the start passes.
            let _ = engine.set_remove_bg_model(&remove_bg_model).await;
            if let Err(e) = engine.start(&model, device, &plugins).await {
                log::warn!("AI engine start: {e}");
            }
        });
    }

    pub fn stop(&mut self) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        self.spawn(async move { engine.stop().await });
    }

    /// The engine row changed: the transport setting applies at once; a new subject model is
    /// switched on the running server; a new model switches the running server (a restart when
    /// the model is not downloaded yet).
    pub fn set_model(&mut self) {
        let model_changed = self.settings.model != self.shown_model;
        let subject_changed = self.settings.remove_bg_model != self.shown_remove_bg_model;
        self.shown_model = self.settings.model.clone();
        self.shown_remove_bg_model = self.settings.remove_bg_model.clone();
        self.shown_transport = self.settings.transport;
        let Some(engine) = self.engine.clone() else {
            return;
        };
        engine.set_transport_mode(self.settings.transport);
        if subject_changed {
            let engine = engine.clone();
            let remove_bg_model = self.settings.remove_bg_model.clone();
            self.spawn(async move {
                if let Err(e) = engine.set_remove_bg_model(&remove_bg_model).await {
                    log::warn!("AI subject model: {e}");
                }
            });
        }
        if !model_changed || !engine.is_running() {
            return;
        }
        let model = self.settings.model.clone();
        let device = self.settings.device;
        let plugins = self.settings.plugins.clone();
        self.spawn(async move {
            if let Err(e) = engine.set_model(&model, device, &plugins).await {
                log::warn!("AI engine model: {e}");
            }
        });
    }

    /// Whether the engine row's settings differ from what the engine was last told.
    pub fn engine_settings_changed(&self) -> bool {
        self.settings.model != self.shown_model
            || self.settings.remove_bg_model != self.shown_remove_bg_model
            || self.settings.transport != self.shown_transport
    }

    /// Asks the console for its image models again.
    pub fn refresh_console(&mut self) {
        if self.console.probe != Probe::Running {
            self.console.probe = Probe::NotStarted;
        }
    }

    /// Once a frame: starts the console probe while the dock is open, takes its answer, and
    /// keeps the settings consistent with what the console lists.
    pub fn poll_console(&mut self) {
        if self.console.signed_out.swap(false, Ordering::AcqRel) {
            self.console.models.clear();
            self.console.probe = Probe::NotStarted;
        }
        if self.open && !self.console.was_open && !self.console.available() {
            self.refresh_console();
        }
        self.console.was_open = self.open;
        if self.open
            && self.console.probe == Probe::NotStarted
            && let Some(client) = self.console.client.clone()
        {
            let (tx, rx) = mpsc::channel();
            let started = self.spawn(async move {
                let _ = tx.send(client.models().await.map_err(|e| e.to_string()));
            });
            if started {
                self.console.rx = Some(rx);
                self.console.probe = Probe::Running;
            } else {
                self.console.probe = Probe::Done;
            }
        }
        let answer = self.console.rx.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(answer) = answer {
            self.console.rx = None;
            self.console.probe = Probe::Done;
            match answer {
                Ok(models) => {
                    if !models.is_empty() {
                        self.push_log(format!(
                            "Redrob Console: {} image model(s)",
                            models.len()
                        ));
                    }
                    self.console.models = models;
                }
                Err(e) => {
                    // Signed out, daemon not running, or a console without image models: the
                    // backend is simply not offered.
                    self.push_log(format!("Redrob Console image models: {e}"));
                    self.console.models.clear();
                }
            }
        }
        if !self.console.available() {
            if self.console.probe == Probe::Done && self.settings.backend == Backend::Console {
                self.settings.backend = Backend::Local;
            }
            return;
        }
        for op in [
            Operation::Fill,
            Operation::Expand,
            Operation::Erase,
            Operation::Generate,
        ] {
            let listed = self.console.models_for(op);
            let current = self.settings.console_model(op).to_owned();
            if !listed.iter().any(|m| m.id == current) {
                *self.settings.console_model_mut(op) =
                    op.default_model(&self.console.models).unwrap_or_default();
            }
        }
    }

    /// Starts `input` as the one running job. `reply` is the agent's pending tool call, answered
    /// when the result is applied.
    pub fn start_job(
        &mut self,
        kind: ToolKind,
        input: JobInput,
        reply: Option<tools::ToolReply>,
    ) -> Result<(), String> {
        if input.uses_console() && !self.console.available() {
            return Err(crate::i18n::t(
                "Redrob Console image models are not available: sign in, then refresh",
            )
            .to_owned());
        }
        if kind.needs_engine() && !input.uses_console() {
            match self.engine_state() {
                EngineState::Ready => {}
                EngineState::Busy => return Err("the AI engine is still working".into()),
                _ => return Err("start the AI engine first".into()),
            }
        }
        let sender = self
            .jobs
            .start(kind, reply)
            .map_err(|running| format!("{} is still running", running.label()))?;
        let engine = self.engine.clone();
        let console = self.console.client.clone().map(|images| ConsoleJob {
            images,
            lang: crate::i18n::current().code,
            signed_out: Arc::clone(&self.console.signed_out),
        });
        let started = self.spawn(async move {
            let output = jobs::run_job(engine, console, input).await;
            sender.send(output);
        });
        if !started {
            self.jobs.cancel();
            return Err("the AI runtime did not start".into());
        }
        self.status = Some((format!("{}…", kind.label()), false));
        self.push_log(format!("{}: started", kind.label()));
        Ok(())
    }

    /// The finished job, if any (the shell applies it through the bridge).
    pub fn poll_job(&mut self) -> Option<JobResult> {
        self.jobs.poll()
    }

    /// Tool calls the agent made since the last frame.
    pub fn take_tool_requests(&mut self) -> Vec<ToolRequest> {
        let mut out = Vec::new();
        while let Ok(request) = self.tool_rx.try_recv() {
            out.push(request);
        }
        out
    }

    /// Lets `snapshot_info` answer without a round trip.
    pub fn refresh_info(&self, mode: Mode, caps: &bridge::Capabilities, size: Option<(u32, u32)>) {
        self.tools.set_info(ModeInfo {
            mode: Some(mode),
            document_size: size.filter(|_| caps.has_document),
            has_selection: caps.has_selection,
            engine_state: self.engine_state().as_str().to_owned(),
        });
    }

    /// Sends `text` to the agent, starting it on the first prompt.
    pub fn agent_send(&mut self, text: String) {
        let text = text.trim().to_owned();
        if text.is_empty() {
            return;
        }
        if self.agent.handle.is_none() {
            let options = AgentOptions {
                system_prompt: Some(SYSTEM_PROMPT.to_owned()),
                ..AgentOptions::default()
            };
            let tools = Arc::clone(&self.tools) as Arc<dyn redrob_ai::agent::ModeTools>;
            match AgentHandle::spawn(tools, options) {
                Ok(handle) => self.agent.handle = Some(handle),
                Err(e) => {
                    let message = e.to_string();
                    self.agent.error = Some(if is_engine_down(&message) {
                        format!("{ENGINE_DOWN} ({message})")
                    } else {
                        message
                    });
                    return;
                }
            }
        }
        self.agent.transcript.push(Bubble {
            speaker: Speaker::User,
            text: text.clone(),
        });
        self.agent.error = None;
        let failed = match &self.agent.handle {
            Some(handle) => handle.prompt(text).err().map(|e| e.to_string()),
            None => None,
        };
        if let Some(message) = failed {
            self.agent.error = Some(message);
            self.agent.handle = None;
        }
    }

    /// Drops the conversation (the next Send starts a new agent).
    pub fn agent_reset(&mut self) {
        self.agent.handle = None;
        self.agent.transcript.clear();
        self.agent.error = None;
        self.agent.busy = false;
    }

    /// Moves the agent's events into the transcript.
    pub fn poll_agent(&mut self) {
        let Some(handle) = &self.agent.handle else {
            return;
        };
        let events = handle.poll();
        let alive = handle.is_alive();
        for event in events {
            match event {
                AgentEvent::Started => self.agent.busy = true,
                AgentEvent::Text(text) => {
                    if !text.trim().is_empty() {
                        self.agent.transcript.push(Bubble {
                            speaker: Speaker::Agent,
                            text,
                        });
                    }
                }
                AgentEvent::ToolCall {
                    name, arguments, ..
                } => {
                    let args = compact_args(&arguments);
                    self.agent.transcript.push(Bubble {
                        speaker: Speaker::Tool,
                        text: format!("{name}({args})"),
                    });
                }
                AgentEvent::ToolResult { name, output, .. } => {
                    let short: String = output.chars().take(200).collect();
                    self.agent.transcript.push(Bubble {
                        speaker: Speaker::Tool,
                        text: format!("{name} → {short}"),
                    });
                }
                AgentEvent::Error(message) => {
                    let text = if is_engine_down(&message) {
                        format!("{ENGINE_DOWN} ({message})")
                    } else {
                        message
                    };
                    self.agent.error = Some(text.clone());
                    self.agent.transcript.push(Bubble {
                        speaker: Speaker::Error,
                        text,
                    });
                }
                AgentEvent::Done => self.agent.busy = false,
            }
        }
        if !alive {
            self.agent.busy = false;
            self.agent.handle = None;
        }
    }
}

/// The IOPaint plugin `kind` needs at server start, if any.
pub fn plugin_for(kind: ToolKind, settings: &ToolSettings) -> Option<&'static str> {
    match kind {
        ToolKind::RemoveBackground => Some("RemoveBG"),
        ToolKind::Upscale => Some("RealESRGAN"),
        ToolKind::FaceRestore => Some(settings.face_plugin),
        ToolKind::SelectSubject => Some(settings.subject_plugin),
        _ => None,
    }
}

fn arg_u32(args: Option<&Value>, key: &str) -> Option<u32> {
    args?.get(key)?.as_u64().and_then(|v| u32::try_from(v).ok())
}

fn arg_str<'a>(args: Option<&'a Value>, key: &str) -> Option<&'a str> {
    args?.get(key)?.as_str()
}

/// Reads everything `kind` needs from the active mode (on the UI thread) so the job can run
/// without it. `args` are the agent's tool arguments; the card's `settings` fill the rest.
pub fn gather_input(
    bridge: &mut dyn bridge::ModeBridge,
    kind: ToolKind,
    settings: &ToolSettings,
    args: Option<&Value>,
) -> Result<JobInput, String> {
    if kind == ToolKind::Generate {
        let prompt = arg_str(args, "prompt")
            .unwrap_or(&settings.generate_prompt)
            .trim()
            .to_owned();
        if prompt.is_empty() {
            return Err("type a prompt".into());
        }
        let size = bridge
            .document_size()
            .filter(|(w, h)| *w > 0 && *h > 0)
            .unwrap_or((1024, 1024));
        return Ok(JobInput::ConsoleGenerate {
            prompt,
            model: console_model(settings, Operation::Generate)?,
            size,
            seed: settings.console_seed(),
        });
    }
    let image = bridge
        .snapshot()
        .ok_or_else(|| "open a document first".to_string())?;
    if image.is_empty() {
        return Err("the document has no pixels".into());
    }
    let selection = || {
        bridge
            .selection_mask()
            .ok_or_else(|| "select the area first".to_string())
    };
    if settings.routes_to_console(kind) {
        return Ok(match kind {
            ToolKind::Erase => JobInput::ConsoleFill {
                mask: selection()?,
                image,
                prompt: None,
                model: console_model(settings, Operation::Erase)?,
                seed: settings.console_seed(),
                erase: true,
            },
            ToolKind::GenerativeFill => {
                let mask = selection()?;
                let prompt = arg_str(args, "prompt")
                    .unwrap_or(&settings.fill_prompt)
                    .trim()
                    .to_owned();
                if prompt.is_empty() {
                    return Err("type a prompt".into());
                }
                JobInput::ConsoleFill {
                    image,
                    mask,
                    prompt: Some(prompt),
                    model: console_model(settings, Operation::Fill)?,
                    seed: settings.console_seed(),
                    erase: false,
                }
            }
            ToolKind::Outpaint => {
                let expand = expand_from(args, settings)?;
                let prompt = arg_str(args, "prompt")
                    .unwrap_or(&settings.prompt)
                    .trim()
                    .to_owned();
                JobInput::ConsoleExpand {
                    image,
                    expand,
                    prompt: (!prompt.is_empty()).then_some(prompt),
                    model: console_model(settings, Operation::Expand)?,
                    seed: settings.console_seed(),
                }
            }
            other => {
                return Err(format!(
                    "{} does not run on Redrob Console",
                    other.label()
                ));
            }
        });
    }
    Ok(match kind {
        ToolKind::Erase => JobInput::Erase {
            mask: selection()?,
            image,
            model: ModelChoice::erase(&settings.model, settings.device, &settings.plugins),
        },
        ToolKind::Inpaint => {
            let mask = selection()?;
            let options =
                settings.inpaint_options(arg_str(args, "prompt"), arg_str(args, "negative_prompt"));
            if !options.is_prompted() {
                return Err("type a prompt".into());
            }
            if !models::is_diffusion_model(&settings.model) {
                return Err("pick a diffusion model (the erase models take no prompt)".into());
            }
            JobInput::Inpaint {
                image,
                mask,
                options,
            }
        }
        ToolKind::GenerativeFill => {
            let mask = selection()?;
            let options =
                settings.fill_options(arg_str(args, "prompt"), arg_str(args, "negative_prompt"));
            if !options.is_prompted() {
                return Err("type a prompt".into());
            }
            let model = settings.fill_model_choice();
            let known_diffusion = models::is_diffusion_model(&model.model);
            if !known_diffusion && models::ERASE_MODELS.contains(&model.model.as_str()) {
                return Err("pick a diffusion model for Generative Fill".into());
            }
            JobInput::GenerativeFill {
                image,
                mask,
                options,
                model,
            }
        }
        ToolKind::Outpaint => {
            let expand = expand_from(args, settings)?;
            JobInput::Outpaint {
                image,
                expand,
                options: settings.inpaint_options(arg_str(args, "prompt"), None),
            }
        }
        ToolKind::RemoveBackground => JobInput::PluginImage {
            plugin: "RemoveBG",
            image,
            scale: 1.0,
            layer_name: redrob_ai::iopaint::client::plugin_layer_name("RemoveBG").to_owned(),
        },
        ToolKind::Upscale => {
            let scale = arg_u32(args, "scale")
                .unwrap_or(settings.upscale)
                .clamp(2, 4);
            JobInput::PluginImage {
                plugin: "RealESRGAN",
                image,
                scale: f64::from(scale),
                layer_name: format!("AI upscale x{scale}"),
            }
        }
        ToolKind::FaceRestore => JobInput::PluginImage {
            plugin: settings.face_plugin,
            image,
            scale: 1.0,
            layer_name: redrob_ai::iopaint::client::plugin_layer_name(settings.face_plugin)
                .to_owned(),
        },
        ToolKind::SelectSubject => JobInput::SelectSubject {
            plugin: settings.subject_plugin,
            image,
        },
        ToolKind::Vectorize => JobInput::Vectorize {
            image,
            preset: arg_str(args, "preset")
                .map(str::to_owned)
                .unwrap_or_else(|| settings.preset.clone()),
        },
        ToolKind::Generate => {
            return Err("Generate runs on Redrob Console".into());
        }
    })
}

/// The sides an expand adds: the agent's arguments, else the card's.
fn expand_from(args: Option<&Value>, settings: &ToolSettings) -> Result<Expand, String> {
    let expand = Expand {
        left: arg_u32(args, "left").unwrap_or(settings.expand.left),
        top: arg_u32(args, "top").unwrap_or(settings.expand.top),
        right: arg_u32(args, "right").unwrap_or(settings.expand.right),
        bottom: arg_u32(args, "bottom").unwrap_or(settings.expand.bottom),
    };
    if !expand.is_valid() {
        return Err(format!(
            "give 1 to {} pixels on at least one side",
            redrob_ai::pixels::MAX_EXPAND
        ));
    }
    Ok(expand)
}

/// The console model the card picked for `op`.
fn console_model(settings: &ToolSettings, op: Operation) -> Result<String, String> {
    let model = settings.console_model(op).trim();
    if model.is_empty() {
        return Err(crate::i18n::t("pick a Redrob Console model").to_owned());
    }
    Ok(model.to_owned())
}

/// `{"a":1,"b":"x"}` as `a: 1, b: x` for a tool bubble.
pub fn compact_args(arguments: &Value) -> String {
    match arguments.as_object() {
        Some(map) if !map.is_empty() => map
            .iter()
            .map(|(k, v)| match v {
                Value::String(s) => format!("{k}: {s}"),
                other => format!("{k}: {other}"),
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn panel_without_a_data_dir_has_no_engine_and_refuses_engine_jobs() {
        let mut panel = AiPanel::new(None);
        assert!(panel.engine().is_none());
        assert_eq!(panel.engine_state(), EngineState::Missing);
        assert!(!panel.is_busy());
        let image = redrob_ai::Rgba8Image::filled(2, 2, [0, 0, 0, 255]).unwrap();
        let mask = redrob_ai::Mask::filled(2, 2, 255).unwrap();
        let model = ModelChoice::erase("lama", Device::Cpu, &[]);
        let err = panel
            .start_job(
                ToolKind::Erase,
                JobInput::Erase { image, mask, model },
                None,
            )
            .unwrap_err();
        assert_eq!(err, "start the AI engine first");
        assert!(
            !panel.jobs.is_busy(),
            "a refused job does not claim the queue"
        );
    }

    #[test]
    fn vectorize_runs_without_the_engine_and_delivers_a_trace() {
        let mut panel = AiPanel::new(None);
        let mut data = Vec::new();
        for y in 0..8u32 {
            for x in 0..8u32 {
                let black = (2..6).contains(&x) && (2..6).contains(&y);
                data.extend_from_slice(if black {
                    &[0, 0, 0, 255]
                } else {
                    &[255, 255, 255, 255]
                });
            }
        }
        let image = redrob_ai::Rgba8Image::new(8, 8, data).unwrap();
        panel
            .start_job(
                ToolKind::Vectorize,
                JobInput::Vectorize {
                    image,
                    preset: "Black and White Logo".into(),
                },
                None,
            )
            .unwrap();
        assert!(panel.jobs.is_busy());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let result = loop {
            if let Some(result) = panel.poll_job() {
                break result;
            }
            assert!(std::time::Instant::now() < deadline, "trace did not finish");
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert_eq!(result.kind, ToolKind::Vectorize);
        match result.output.unwrap() {
            jobs::JobOutput::Trace { result, size, .. } => {
                assert_eq!(size, (8, 8));
                assert!(!result.paths.is_empty());
            }
            other => panic!("{other:?}"),
        }
        assert!(!panel.jobs.is_busy());
    }

    #[test]
    fn settings_build_inpaint_options() {
        let settings = ToolSettings {
            prompt: " a bench ".into(),
            steps: 30,
            ..ToolSettings::default()
        };
        let options = settings.inpaint_options(None, None);
        assert_eq!(options.prompt.as_deref(), Some("a bench"));
        assert_eq!(options.negative_prompt, None);
        assert_eq!(options.sd_steps, Some(30));
        assert_eq!(options.sd_keep_unmasked_area, Some(true));
        let overridden = settings.inpaint_options(Some("a tree"), Some("blur"));
        assert_eq!(overridden.prompt.as_deref(), Some("a tree"));
        assert_eq!(overridden.negative_prompt.as_deref(), Some("blur"));
        assert!(redrob_ai::vectorize::preset_names().contains(&settings.preset.as_str()));
        // Generative Fill has its own prompt and model; sampling settings are shared.
        let fill = settings.fill_options(None, None);
        assert_eq!(fill.prompt, None);
        assert_eq!(fill.sd_steps, Some(30));
        let fill = ToolSettings {
            fill_prompt: " a red door ".into(),
            ..ToolSettings::default()
        };
        let options = fill.fill_options(None, Some("blur"));
        assert_eq!(options.prompt.as_deref(), Some("a red door"));
        assert_eq!(options.negative_prompt.as_deref(), Some("blur"));
        assert_eq!(options.sd_keep_unmasked_area, Some(true));
        assert_eq!(fill.fill_model_choice().model, models::DEFAULT_FILL_MODEL);
        assert_eq!(fill.transport, TransportMode::File);
        assert_eq!(fill.remove_bg_model, models::DEFAULT_REMOVE_BG_MODEL);
    }

    /// A pixel-mode bridge over one image, with or without a selection.
    struct FakeBridge {
        image: redrob_ai::Rgba8Image,
        selection: Option<redrob_ai::Mask>,
    }

    impl bridge::ModeBridge for FakeBridge {
        fn mode(&self) -> Mode {
            Mode::Photo
        }
        fn snapshot(&mut self) -> Option<redrob_ai::Rgba8Image> {
            Some(self.image.clone())
        }
        fn selection_mask(&self) -> Option<redrob_ai::Mask> {
            self.selection.clone()
        }
        fn place_layer(
            &mut self,
            _: &str,
            _: redrob_ai::Rgba8Image,
            _: (i32, i32),
        ) -> bridge::BridgeResult<()> {
            Ok(())
        }
        fn set_selection(&mut self, mask: redrob_ai::Mask) -> bridge::BridgeResult<()> {
            self.selection = Some(mask);
            Ok(())
        }
        fn expand_canvas(&mut self, _: Expand) -> bridge::BridgeResult<()> {
            Ok(())
        }
        fn document_size(&self) -> Option<(u32, u32)> {
            Some((self.image.width, self.image.height))
        }
        fn supports_selection(&self) -> bool {
            true
        }
        fn supports_place_layer(&self) -> bool {
            true
        }
        fn supports_expand(&self) -> bool {
            true
        }
    }

    #[test]
    fn gathering_inputs_reads_the_bridge_and_the_arguments() {
        let image = redrob_ai::Rgba8Image::filled(3, 2, [9, 9, 9, 255]).unwrap();
        let mut fake = FakeBridge {
            image: image.clone(),
            selection: None,
        };
        let mut settings = ToolSettings::default();
        assert_eq!(
            gather_input(&mut fake, ToolKind::Erase, &settings, None).unwrap_err(),
            "select the area first"
        );
        fake.selection = Some(redrob_ai::Mask::filled(3, 2, 255).unwrap());
        match gather_input(&mut fake, ToolKind::Erase, &settings, None).unwrap() {
            JobInput::Erase { model, .. } => assert_eq!(model.model, "lama"),
            other => panic!("{other:?}"),
        }
        // Generative Fill needs a prompt (its own, or the agent's) and holds the fill model.
        assert_eq!(
            gather_input(&mut fake, ToolKind::GenerativeFill, &settings, None).unwrap_err(),
            "type a prompt"
        );
        let args = serde_json::json!({ "prompt": "a red door" });
        match gather_input(&mut fake, ToolKind::GenerativeFill, &settings, Some(&args)).unwrap() {
            JobInput::GenerativeFill { options, model, .. } => {
                assert_eq!(options.prompt.as_deref(), Some("a red door"));
                assert_eq!(model.model, models::DEFAULT_FILL_MODEL);
            }
            other => panic!("{other:?}"),
        }
        settings.fill_model = "lama".into();
        assert!(
            gather_input(&mut fake, ToolKind::GenerativeFill, &settings, Some(&args))
                .unwrap_err()
                .starts_with("pick a diffusion model")
        );
        settings.fill_model = models::DEFAULT_FILL_MODEL.to_owned();
        // Inpaint needs a prompt and a diffusion model.
        assert_eq!(
            gather_input(&mut fake, ToolKind::Inpaint, &settings, None).unwrap_err(),
            "type a prompt"
        );
        let args = serde_json::json!({ "prompt": "a tree" });
        assert!(
            gather_input(&mut fake, ToolKind::Inpaint, &settings, Some(&args))
                .unwrap_err()
                .starts_with("pick a diffusion model")
        );
        settings.model = models::DIFFUSION_MODELS[0].to_owned();
        match gather_input(&mut fake, ToolKind::Inpaint, &settings, Some(&args)).unwrap() {
            JobInput::Inpaint { options, .. } => {
                assert_eq!(options.prompt.as_deref(), Some("a tree"));
            }
            other => panic!("{other:?}"),
        }
        // Outpaint takes its sides from the arguments, else the card.
        let args = serde_json::json!({ "left": 10, "right": 0 });
        match gather_input(&mut fake, ToolKind::Outpaint, &settings, Some(&args)).unwrap() {
            JobInput::Outpaint { expand, .. } => {
                assert_eq!(expand.left, 10);
                assert_eq!(expand.right, 0);
                assert_eq!(expand.top, settings.expand.top);
            }
            other => panic!("{other:?}"),
        }
        // Upscale clamps the scale; Vectorize takes the preset from the arguments.
        let args = serde_json::json!({ "scale": 9 });
        match gather_input(&mut fake, ToolKind::Upscale, &settings, Some(&args)).unwrap() {
            JobInput::PluginImage { scale, plugin, .. } => {
                assert_eq!(scale, 4.0);
                assert_eq!(plugin, "RealESRGAN");
            }
            other => panic!("{other:?}"),
        }
        let args = serde_json::json!({ "preset": "Flat Logo" });
        match gather_input(&mut fake, ToolKind::Vectorize, &settings, Some(&args)).unwrap() {
            JobInput::Vectorize { preset, image: got } => {
                assert_eq!(preset, "Flat Logo");
                assert_eq!(got, image);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(plugin_for(ToolKind::FaceRestore, &settings), Some("GFPGAN"));
        assert_eq!(plugin_for(ToolKind::Erase, &settings), None);
        assert_eq!(plugin_for(ToolKind::GenerativeFill, &settings), None);
    }

    #[test]
    fn engine_down_is_recognised_and_args_are_compacted() {
        assert!(is_engine_down(
            "The Redrob Code engine could not be started: x"
        ));
        assert!(!is_engine_down("tool panicked"));
        assert_eq!(
            compact_args(&serde_json::json!({ "prompt": "a tree", "scale": 2 })),
            "prompt: a tree, scale: 2"
        );
        assert_eq!(compact_args(&serde_json::json!({})), "");
    }
}
