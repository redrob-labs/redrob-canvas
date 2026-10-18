//! The Redrob Canvas shell: one window, a thin top bar, and the three upstream editors hosted as
//! switchable modes. Editors are created lazily on their first switch and then kept alive, so
//! switching modes preserves their open documents. The shell owns the egui context's fonts and
//! style ([`ShellApp::setup_context`]): the fonts are built once, at start-up, with every
//! editor's families, and the style is the Photo editor's, which every mode draws with. The shell also owns what crosses the modes:
//! the Mode menu's hand-offs (`handoff`), file routing by extension (`routing`) for Open and
//! drops, the window title, the exit confirm for editors that are not on screen, and the one
//! interface language (`i18n`), pushed into each editor's own preference.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use redrob_layout_ui_egui::DesignApp;
use redrob_photo_ui_egui::PhotocraftApp;
use redrob_vector_ui_egui::VectorcraftApp;
use serde_json::{Value, json};

use crate::ai::bridge::{Capabilities, LayoutBridge, ModeBridge, PhotoBridge, VectorBridge};
use crate::ai::jobs::{JobOutput, JobResult, ToolKind};
use crate::ai::tools::{self, ToolReply, ToolRequest};
use crate::ai::{self, Action, AiPanel};
use crate::handoff::Handoff;
use crate::i18n::{self, t, tf};
use crate::routing::{self, Route};
use crate::{handoff, services, trace};

mod app_bridge;

/// Height of the shell's own top bar, in points.
pub const TOP_BAR_HEIGHT: f32 = 28.0;

/// Ctrl+Shift+O: the shell's Open (routes the file to its editor), whichever editor is active.
pub const OPEN_ANY_SHORTCUT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(
    egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
    egui::Key::O,
);

/// Ctrl+O: the shell's Open while the shell owns the keyboard ([`ShellApp::shell_owns_focus`]);
/// otherwise the active editor's own Open.
pub const OPEN_SHORTCUT: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);

/// How long a status line stays before it has faded out, in seconds.
pub const STATUS_SECS: f32 = 5.0;
/// A status line with a button ("Open in Layout instead") stays longer.
pub const STATUS_WITH_ACTION_SECS: f32 = 20.0;

/// The About window's lockup: the Primary lockup with "Canvas", for a light ground, built by
/// scripts/build-brand-assets.py from the design system and pinned in brand/SOURCES.json.
const ABOUT_LOCKUP_PNG: &[u8] = include_bytes!("../../../resources/brand/redrob-canvas-lockup.png");
/// The lockup's height in the About window (above the 18px minimum in 10-logo.md).
const ABOUT_LOCKUP_HEIGHT: f32 = 40.0;
/// The editors this app is built on (UPSTREAM_NOTICES.md); translated through `i18n`.
const ABOUT_ATTRIBUTION: &str =
    "MIT OR Apache-2.0. Third-party notices: NOTICE and THIRD_PARTY_NOTICES.md.";

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

    /// The mode's English name: the identifier in tool replies, logs and file filters.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Photo => "Photo",
            Mode::Vector => "Vector",
            Mode::Layout => "Layout",
        }
    }

    /// The segmented button's label, in the interface language.
    pub fn name(self) -> &'static str {
        t(self.label())
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

/// The window title: `<active document name>[*] - Redrob Canvas`, or the product name alone
/// with no document open. `dirty` adds the `*`.
pub fn window_title(doc: Option<(&str, bool)>) -> String {
    match doc {
        Some((name, dirty)) => format!(
            "{name}{} - {}",
            if dirty { "*" } else { "" },
            services::APP_NAME
        ),
        None => services::APP_NAME.to_string(),
    }
}

/// The file name of `path` for a message, or the path itself when it has none.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

/// A button on a status line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusAction {
    /// Open the file in this mode too ("Open in Layout instead" for a PDF).
    OpenIn(Mode, PathBuf),
}

/// The shell's status line in the top bar: one line that fades out after [`STATUS_SECS`].
#[derive(Clone, Debug)]
pub struct Status {
    pub text: String,
    pub error: bool,
    pub action: Option<StatusAction>,
    since: Instant,
}

impl Status {
    pub fn new(text: impl Into<String>, error: bool) -> Self {
        Self {
            text: text.into(),
            error,
            action: None,
            since: Instant::now(),
        }
    }

    /// How long this line stays in all.
    fn lifetime(&self) -> f32 {
        if self.action.is_some() {
            STATUS_WITH_ACTION_SECS
        } else {
            STATUS_SECS
        }
    }

    /// The line's opacity at `now`: full, then fading through its last second; `None` once it has
    /// gone.
    pub fn alpha(&self, now: Instant) -> Option<f32> {
        let left = self.lifetime() - now.saturating_duration_since(self.since).as_secs_f32();
        (left > 0.0).then_some(left.min(1.0))
    }
}

/// What the exit confirm's buttons asked for this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExitChoice {
    None,
    Cancel,
    GoTo(Mode),
    Quit,
}

/// The font families every editor draws with, added to the Photo editor's font definitions
/// (the shell's): the Vector and Layout editors' own families (`ui`, `ui-semibold`, `mono`,
/// `arabic`...) and the font files they name. The generic families (`Proportional`,
/// `Monospace`) and a name both sides define stay the Photo editor's.
fn merge_extra_fonts(into: &mut egui::FontDefinitions, extra: &egui::FontDefinitions) {
    for (name, data) in &extra.font_data {
        into.font_data
            .entry(name.clone())
            .or_insert_with(|| data.clone());
    }
    for (family, stack) in &extra.families {
        if matches!(
            family,
            egui::FontFamily::Proportional | egui::FontFamily::Monospace
        ) {
            continue;
        }
        into.families
            .entry(family.clone())
            .or_insert_with(|| stack.clone());
    }
}

/// The other editors' font definitions, for [`merge_extra_fonts`].
fn extra_fonts() -> Vec<egui::FontDefinitions> {
    vec![
        redrob_vector_ui_egui::theme::font_definitions(),
        redrob_layout_ui_egui::theme::ui_font_definitions("").0,
    ]
}

/// Whether the live font definitions hold every family in `extra` (the Photo editor's own
/// re-install, after a UI font size change, drops them).
fn has_families(live: &egui::FontDefinitions, extra: &[egui::FontDefinitions]) -> bool {
    extra
        .iter()
        .flat_map(|d| d.families.keys())
        .all(|family| live.families.contains_key(family))
}

/// The eframe app: the shell plus its (lazily created) editors.
pub struct ShellApp {
    mode: Mode,
    /// A switch asked for (button, shortcut, `--mode`) and applied at the next frame's start,
    /// once the shared fonts hold every editor's families.
    pending: Option<Mode>,
    /// The Vector and Layout editors' font definitions, merged into the shell's fonts at
    /// start-up and again should the Photo editor re-install its own ([`has_families`]).
    extra_fonts: Vec<egui::FontDefinitions>,
    /// [`Self::setup_context`] ran: the fonts hold every editor's families from the first frame.
    context_ready: bool,
    photo: Option<PhotocraftApp>,
    vector: Option<VectorcraftApp>,
    layout: Option<DesignApp>,
    /// The window's wgpu state, handed to the Photo editor's GPU canvas when it is created.
    wgpu: Option<eframe::egui_wgpu::RenderState>,
    /// Modes whose editor has been created, in creation order (for tests and the status line).
    created: Vec<Mode>,
    /// Modes whose editor was told the host owns the context ([`Self::adopt_context`]).
    adopted: Vec<Mode>,
    /// The AI dock, menu, engine, job queue and agent.
    pub ai: AiPanel,
    /// What the active mode's bridge could do when this frame's logic ran.
    caps: Capabilities,
    /// The shell's status line (hand-off results, routing errors), fading.
    status: Option<Status>,
    /// Files dropped on the window that belong to another mode, routed at the next frame's start.
    pending_opens: Vec<(PathBuf, Route)>,
    /// Shift state from the last frame; `RawInput` no longer carries modifiers in egui 0.36,
    /// and `raw_input_hook` runs before the frame's input is assembled.
    shift_held: bool,
    /// The exit confirm: every editor's unsaved-document count, while the dialog is up.
    exit_prompt: Option<Vec<(Mode, usize)>>,
    /// "Quit anyway" was clicked: the next close request passes to the active editor.
    exit_approved: bool,
    /// The window title the shell set last.
    last_title: Option<String>,
    /// Help > About is open.
    about_open: bool,
    /// The About window's lockup, uploaded on first use.
    about_lockup: Option<egui::TextureHandle>,
    /// The one interface language: the persisted preference against the system's.
    language: i18n::Language,
    /// The language value last written into each editor's own preference, so a change the editor
    /// made itself (its own Language menu) is told apart from one the shell made.
    pushed_language: HashMap<Mode, String>,
    /// When the last switch was asked for and how many frames ran since it applied (trace).
    switch_asked: Option<Instant>,
    frames_since_switch: u32,
    /// `REDROB_CANVAS_TRACE_SWITCH=vector,layout,photo`: switch to each in turn, 40 frames apart
    /// (a measurement that needs no input device), then quit.
    trace_switches: Vec<Mode>,
    frame_count: u32,
    /// The Redrob Code app bridge ([`app_bridge`]): the tools the shared daemon's agent drives
    /// this window with. `None` until `main` starts it and in tests.
    bridge: Option<app_bridge::Bridge>,
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
            extra_fonts: Vec::new(),
            context_ready: false,
            photo: None,
            vector: None,
            layout: None,
            wgpu,
            created: Vec::new(),
            adopted: Vec::new(),
            ai,
            caps: Capabilities::none(Mode::default()),
            status: None,
            pending_opens: Vec::new(),
            shift_held: false,
            exit_prompt: None,
            exit_approved: false,
            last_title: None,
            about_open: false,
            about_lockup: None,
            language: i18n::Language::load(),
            pushed_language: HashMap::new(),
            switch_asked: None,
            frames_since_switch: u32::MAX,
            trace_switches: std::env::var("REDROB_CANVAS_TRACE_SWITCH")
                .ok()
                .map(|v| v.split(',').filter_map(crate::parse_mode).rev().collect())
                .unwrap_or_default(),
            frame_count: 0,
            bridge: None,
        }
    }

    /// The interface language.
    #[cfg_attr(not(test), allow(dead_code))] // exercised by the unit tests
    pub fn language(&self) -> &'static i18n::LangInfo {
        self.language.effective()
    }

    /// Help > Language: `auto` or a code. Every existing editor takes it now, whatever it showed
    /// (a choice in the shell outranks an editor's own), and new editors take it when created.
    pub fn set_language(&mut self, pref: &str) {
        self.language.set(pref);
        let lang = self.language.effective();
        i18n::set_current(lang);
        self.pushed_language.clear();
        for mode in Mode::ALL {
            if self.editor_language(mode).is_some() {
                self.write_editor_language(mode, lang);
                self.pushed_language
                    .insert(mode, Self::editor_value(mode, lang));
            }
        }
    }

    /// The language value an editor's own preference holds now (`auto` or a code for Photo and
    /// Vector, the Layout code for Layout); `None` while the editor does not exist.
    fn editor_language(&self, mode: Mode) -> Option<String> {
        match mode {
            Mode::Photo => self
                .photo
                .as_ref()
                .map(|a| a.session.prefs().interface.language.clone()),
            Mode::Vector => self
                .vector
                .as_ref()
                .map(|a| a.session.prefs.interface_language.clone()),
            Mode::Layout => self.layout.as_ref().map(|a| a.ui.language.clone()),
        }
    }

    /// What `mode`'s preference should hold for `lang`.
    fn editor_value(mode: Mode, lang: &i18n::LangInfo) -> String {
        match mode {
            Mode::Photo | Mode::Vector => lang.code.to_owned(),
            Mode::Layout => lang.layout.to_owned(),
        }
    }

    /// The shell preference an editor's own value means: `auto`, a common code, or `None` for a
    /// language not every editor has (the editor keeps it; the shell does not fight it).
    fn shell_pref_for(mode: Mode, value: &str) -> Option<String> {
        match mode {
            Mode::Photo | Mode::Vector => {
                if value.eq_ignore_ascii_case(i18n::AUTO) {
                    Some(i18n::AUTO.to_owned())
                } else {
                    i18n::by_code(value).map(|l| l.code.to_owned())
                }
            }
            Mode::Layout => i18n::by_layout_code(value).map(|l| l.code.to_owned()),
        }
    }

    /// Writes `lang` into `mode`'s own preference (the editor's Language menu and preferences
    /// file follow: photocraft `interface.language`, vectorcraft `interfaceLanguage`,
    /// designcraft `UiState::language`).
    fn write_editor_language(&mut self, mode: Mode, lang: &i18n::LangInfo) {
        let value = Self::editor_value(mode, lang);
        let result = match mode {
            Mode::Photo => self.photo.as_mut().map(|app| {
                app.run(
                    "prefs.set",
                    json!({ "values": { "interface.language": value } }),
                )
            }),
            Mode::Vector => self.vector.as_mut().map(|app| {
                app.run(
                    "prefs.set",
                    json!({ "key": "interfaceLanguage", "value": value }),
                )
            }),
            Mode::Layout => self.layout.as_mut().map(|app| {
                app.ui.language = value;
                Ok(Value::Null)
            }),
        };
        if let Some(Err(e)) = result {
            log::warn!("{}: couldn't set the language: {e}", mode.label());
        }
    }

    /// One language everywhere: every existing editor's preference is made the shell's, and a
    /// language an editor chose on its own (its menu) becomes the shell's for all of them.
    fn sync_language(&mut self) {
        // First: a value an editor changed on its own since the shell last wrote it is adopted.
        for mode in Mode::ALL {
            let Some(current) = self.editor_language(mode) else {
                continue;
            };
            if self
                .pushed_language
                .get(&mode)
                .is_some_and(|p| *p != current)
                && let Some(pref) = Self::shell_pref_for(mode, &current)
                && self.language.set(&pref)
            {
                i18n::set_current(self.language.effective());
            }
        }
        // Then: every editor holds the shell's language, except one showing a language not all of
        // them have (left as it is, and remembered so it is not taken for a new choice).
        let lang = self.language.effective();
        for mode in Mode::ALL {
            let Some(current) = self.editor_language(mode) else {
                continue;
            };
            if Self::shell_pref_for(mode, &current).is_none() {
                self.pushed_language.insert(mode, current);
                continue;
            }
            let want = Self::editor_value(mode, lang);
            if current != want {
                self.write_editor_language(mode, lang);
            }
            self.pushed_language.insert(mode, want);
        }
    }

    /// Help > Language: the system's language, then every language all three editors have.
    fn language_menu(&mut self, ui: &mut egui::Ui) {
        let system = self.language.system();
        let auto = ui
            .radio(
                self.language.is_auto(),
                tf("System ({name})", &[("name", system.name)]),
            )
            .on_hover_text(t("Follow the operating system's language"));
        if auto.clicked() {
            ui.close();
            self.set_language(i18n::AUTO);
        }
        ui.separator();
        let effective = self.language.effective();
        for lang in i18n::LANGUAGES {
            let selected = !self.language.is_auto() && effective.code == lang.code;
            let item = ui
                .radio(selected, lang.name)
                .on_hover_text(t("The same language in Photo, Vector and Layout"));
            if item.clicked() {
                ui.close();
                self.set_language(lang.code);
            }
        }
    }

    /// Prepare the egui context once, before the first frame: the Photo editor's plugins, image
    /// loaders, theme and fonts, with the other editors' font families merged into the one font
    /// set, and the other editors told that the host owns the style. After this no editor
    /// installs fonts or resets the global style, and a switch to any mode draws at once.
    pub fn setup_context(&mut self, ctx: &egui::Context) {
        redrob_vector_ui_egui::theme::set_host_owns_style(true);
        redrob_layout_ui_egui::theme::set_host_owns_style(true);
        self.extra_fonts = extra_fonts();
        let theme = self.ensure_photo().ui.theme;
        // The Photo editor's first frame re-installs its fonts when the context does not yet
        // carry its language; mark it now, so the one install below is the only one. (Its own
        // install here is replaced by the merged set before either lands: a pending `set_fonts`
        // is one slot.)
        let language = self
            .ensure_photo()
            .session
            .prefs()
            .interface
            .language
            .clone();
        redrob_photo_ui_egui::i18n::sync_context(ctx, &language);
        let extra = self.extra_fonts.clone();
        PhotocraftApp::setup_context_with_fonts(ctx, theme, |fonts| {
            for defs in &extra {
                merge_extra_fonts(fonts, defs);
            }
        });
        if let Some(photo) = self.photo.as_mut() {
            photo.context_ready();
        }
        self.adopted.push(Mode::Photo);
        self.context_ready = true;
        trace::log(|| {
            "shell set up the context: fonts built once with every mode's families".to_string()
        });
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
        if mode != self.mode {
            trace::log(|| format!("switch asked: {} -> {}", self.mode.label(), mode.label()));
            self.switch_asked = Some(Instant::now());
        }
        self.pending = Some(mode);
    }

    /// The mode asked for and not applied yet.
    pub fn pending_mode(&self) -> Option<Mode> {
        self.pending
    }

    /// Apply a requested switch. The fonts hold every editor's families from the first frame
    /// ([`Self::setup_context`]), so the new editor draws in this very frame; only after the Photo
    /// editor re-installed its own fonts does the switch wait a pass for [`Self::sync_fonts`].
    pub fn apply_pending(&mut self, ctx: &egui::Context) {
        let Some(next) = self.pending else { return };
        if next == self.mode {
            self.pending = None;
            return;
        }
        if !self.fonts_settled(ctx) {
            trace::log(|| "switch waits: fonts not settled".to_string());
            return;
        }
        self.pending = None;
        trace::log(|| {
            format!(
                "switch applied: {} -> {} ({:.1} ms after the ask)",
                self.mode.label(),
                next.label(),
                self.switch_asked
                    .map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0)
            )
        });
        self.frames_since_switch = 0;
        self.mode = next;
    }

    /// Whether the live font definitions hold every editor's families.
    fn fonts_settled(&self, ctx: &egui::Context) -> bool {
        !self.context_ready || ctx.fonts(|f| has_families(f.definitions(), &self.extra_fonts))
    }

    /// The Photo editor re-installs its fonts on its own when the UI font size or language
    /// changes, replacing the shell's set: merge the other editors' families back in. A no-op
    /// every other frame (one look at the family names).
    fn sync_fonts(&mut self, ctx: &egui::Context) {
        if !self.context_ready || self.fonts_settled(ctx) {
            return;
        }
        let mut defs = ctx.fonts(|f| f.definitions().clone());
        for extra in &self.extra_fonts {
            merge_extra_fonts(&mut defs, extra);
        }
        trace::log(|| {
            format!(
                "shell set_fonts (re-merge after a Photo re-install): {} fonts, {} families",
                defs.font_data.len(),
                defs.families.len()
            )
        });
        ctx.set_fonts(defs);
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
                            .selectable_label(self.mode == mode, mode.name())
                            .on_hover_text(format!(
                                "{}  ({})",
                                mode.name(),
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
                    ui.menu_button(t("Mode"), |ui| self.mode_menu(ui));
                    ui.separator();
                    ai::panel::menu(ui, &mut self.ai, &caps);
                    ui.menu_button(t("Help"), |ui| {
                        if ui
                            .button(tf("About {app}", &[("app", services::APP_NAME)]))
                            .clicked()
                        {
                            ui.close();
                            self.about_open = true;
                        }
                        ui.menu_button(t("Language"), |ui| self.language_menu(ui));
                        crate::update::menu_items(ui);
                    });
                    self.status_line(ui);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        self.right_slot(ui);
                    });
                });
            });
    }

    /// The "Mode" menu: the shell's Open, and Send to Photo / Vector / Layout, each enabled when
    /// the active mode has a document and a hand-off to that mode ([`Handoff::between`]).
    fn mode_menu(&mut self, ui: &mut egui::Ui) {
        let open = ui.button(t("Open…")).on_hover_text(format!(
            "{}  ({})",
            t("Open a file in the editor its type belongs to"),
            ui.ctx().format_shortcut(&OPEN_ANY_SHORTCUT)
        ));
        if open.clicked() {
            ui.close();
            self.open_dialog();
        }
        ui.separator();
        let source = self.mode;
        let has_document = self.caps.has_document;
        for target in Mode::ALL {
            let handoff = Handoff::between(source, target);
            let enabled = has_document && handoff.is_some();
            let button = ui.add_enabled(
                enabled,
                egui::Button::new(tf("Send to {mode}", &[("mode", target.name())])),
            );
            let button = match (handoff, has_document) {
                (Some(h), true) => button.on_hover_text(h.description()),
                (Some(_), false) => button.on_disabled_hover_text(t("open a document first")),
                (None, _) => button.on_disabled_hover_text(t("this is the active mode")),
            };
            if button.clicked() {
                ui.close();
                self.send_to(target);
            }
        }
    }

    /// Runs the hand-off from the active mode to `target` and reports on the status line.
    pub fn send_to(&mut self, target: Mode) {
        match handoff::send_to(self, target) {
            Ok(line) => self.set_status(line, false),
            Err(e) => self.set_status(
                tf(
                    "Send to {mode}: {error}",
                    &[("mode", target.name()), ("error", &e)],
                ),
                true,
            ),
        }
    }

    /// Show `text` on the status line (an error in the error colour) for [`STATUS_SECS`].
    pub fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some(Status::new(text, error));
    }

    /// The status line, if one is showing.
    #[cfg_attr(not(test), allow(dead_code))] // exercised by the unit tests
    pub fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }

    /// The top bar's status line: the last hand-off or routing result, fading out, with its
    /// button when it has one.
    fn status_line(&mut self, ui: &mut egui::Ui) {
        let Some(status) = self.status.clone() else {
            return;
        };
        let Some(alpha) = status.alpha(Instant::now()) else {
            self.status = None;
            return;
        };
        ui.add_space(8.0);
        let color = if status.error {
            ui.visuals().error_fg_color
        } else {
            ui.visuals().text_color()
        };
        ui.label(egui::RichText::new(&status.text).color(color.gamma_multiply(alpha)));
        if let Some(StatusAction::OpenIn(mode, path)) = &status.action
            && ui
                .small_button(tf("Open in {mode} instead", &[("mode", mode.name())]))
                .clicked()
        {
            self.status = None;
            self.open_in(*mode, path, None);
            return;
        }
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }

    /// The right side of the top bar: the AI dock toggle.
    pub fn right_slot(&mut self, ui: &mut egui::Ui) {
        ai::panel::toggle(ui, &mut self.ai);
        crate::update::top_bar(ui);
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

    /// The Photo editor (created on demand): where a rasterising hand-off lands.
    pub fn photo_app(&mut self) -> &mut PhotocraftApp {
        self.ensure_photo()
    }

    /// The Layout editor (created on demand): where a placing hand-off lands.
    pub fn layout_app(&mut self) -> &mut DesignApp {
        self.ensure_layout()
    }

    /// The shell's Open: a file dialog filtered by every editor's extensions, each chosen file
    /// routed to its editor ([`Self::open_paths`]).
    pub fn open_dialog(&mut self) {
        let all: Vec<&str> = Mode::ALL
            .iter()
            .flat_map(|m| routing::extensions_for(*m).iter().copied())
            .collect();
        let mut dialog = rfd::FileDialog::new()
            .set_title(t("Open in Redrob Canvas"))
            .add_filter(t("All documents"), all.as_slice());
        for mode in Mode::ALL {
            dialog = dialog.add_filter(mode.name(), routing::extensions_for(mode));
        }
        if let Some(paths) = dialog.pick_files() {
            self.open_paths(paths);
        }
    }

    /// Opens each path in the editor its extension belongs to ([`routing::route`]); a file no
    /// editor claims is reported on the status line.
    pub fn open_paths(&mut self, paths: Vec<PathBuf>) {
        for path in paths {
            match routing::route(&path) {
                Some(route) => self.open_in(route.mode, &path, route.alternative),
                None => self.set_status(
                    tf(
                        "{name}: no editor opens this kind of file",
                        &[("name", &file_name(&path))],
                    ),
                    true,
                ),
            }
        }
    }

    /// Opens `path` with `mode`'s editor's own open command and switches to that mode. With an
    /// `alternative` mode, the status line offers to open the file there instead.
    pub fn open_in(&mut self, mode: Mode, path: &Path, alternative: Option<Mode>) {
        let name = file_name(path);
        match self.editor_open(mode, path) {
            Ok(()) => {
                self.set_mode(mode);
                let mut status = Status::new(
                    tf(
                        "Opened {name} in {mode}",
                        &[("name", &name), ("mode", mode.name())],
                    ),
                    false,
                );
                if let Some(alt) = alternative {
                    status.action = Some(StatusAction::OpenIn(alt, path.to_path_buf()));
                }
                self.status = Some(status);
            }
            Err(e) => self.set_status(format!("{name}: {e}"), true),
        }
    }

    /// Each editor's own open path, so the file is named, remembered in Open Recent and imported
    /// exactly as it would be from that editor's File menu: `PhotocraftApp::open_path`
    /// (crafts/photocraft/crates/ui-egui/src/file_open.rs L103; a background job when the editor
    /// runs them), `redrob_vector_ui_egui::io::open_path` (crafts/vectorcraft/crates/ui-egui/src/
    /// io.rs L218) and `DesignApp::open_dropped` (crafts/designcraft/crates/ui-egui/src/lib.rs
    /// L1140: layouts open, text and graphics are placed).
    fn editor_open(&mut self, mode: Mode, path: &Path) -> Result<(), String> {
        let path = path.to_string_lossy().to_string();
        match mode {
            Mode::Photo => self.ensure_photo().open_path(&path).map(|_warnings| ()),
            Mode::Vector => {
                redrob_vector_ui_egui::io::open_path(self.ensure_vector(), &path).map(|_| ())
            }
            Mode::Layout => self.ensure_layout().open_dropped(&path).map(|_| ()),
        }
    }

    /// Files dropped on the window that belong to another mode ([`Self::route_dropped_files`]),
    /// opened now.
    fn flush_pending_opens(&mut self) {
        for (path, route) in std::mem::take(&mut self.pending_opens) {
            self.open_in(route.mode, &path, route.alternative);
        }
    }

    /// Dropped files whose extension belongs to another mode are taken from the input before the
    /// active editor sees it and opened there; the rest (the active mode's own files, unknown
    /// types, and everything while Shift is held) stay for the editor's own drop handling, which
    /// may place rather than open them.
    fn route_dropped_files(&mut self, raw: &mut egui::RawInput) {
        if raw.dropped_files.is_empty() || self.shift_held {
            return;
        }
        let dropped = std::mem::take(&mut raw.dropped_files);
        let mut keep = Vec::with_capacity(dropped.len());
        for file in dropped {
            match routing::route(file.path()) {
                Some(route) if route.mode != self.mode => {
                    self.pending_opens.push((file.path().to_path_buf(), route));
                }
                _ => keep.push(file),
            }
        }
        raw.dropped_files = keep;
    }

    /// Whether Ctrl+O is the shell's rather than the active editor's: while the exit confirm is
    /// up, or while the active editor has no document (its Open would open its own kinds only;
    /// the shell's routes every kind).
    fn shell_owns_focus(&self) -> bool {
        self.exit_prompt.is_some() || !self.caps.has_document
    }

    /// The active mode's document name and whether it has unsaved changes, read from the
    /// editor's session (`DocState::is_dirty`: crafts/photocraft/crates/engine/src/lib.rs L291,
    /// crafts/vectorcraft/crates/engine/src/lib.rs L419, crafts/designcraft/crates/engine/src/
    /// lib.rs L155; `DocState::title` vectorcraft L479, designcraft L158).
    pub fn active_document(&self) -> Option<(String, bool)> {
        match self.mode {
            Mode::Photo => self
                .photo
                .as_ref()?
                .session
                .active()
                .map(|st| (st.doc.name.clone(), st.is_dirty())),
            Mode::Vector => self
                .vector
                .as_ref()?
                .session
                .active()
                .map(|st| (st.title(), st.is_dirty())),
            Mode::Layout => self
                .layout
                .as_ref()?
                .session
                .active()
                .map(|st| (st.title(), st.is_dirty())),
        }
    }

    /// Every editor's count of documents with unsaved changes (modes with none left out).
    pub fn dirty_counts(&self) -> Vec<(Mode, usize)> {
        let mut out = Vec::new();
        if let Some(app) = &self.photo {
            let n = app
                .session
                .documents()
                .iter()
                .filter(|d| d.is_dirty())
                .count();
            if n > 0 {
                out.push((Mode::Photo, n));
            }
        }
        if let Some(app) = &self.vector {
            let n = app
                .session
                .documents()
                .iter()
                .filter(|d| d.is_dirty())
                .count();
            if n > 0 {
                out.push((Mode::Vector, n));
            }
        }
        if let Some(app) = &self.layout {
            let n = app
                .session
                .documents()
                .iter()
                .filter(|d| d.is_dirty())
                .count();
            if n > 0 {
                out.push((Mode::Layout, n));
            }
        }
        out
    }

    /// The window title is the shell's ([`window_title`]). Each editor sends its own on a document
    /// change (`*name — <editor>`); the window reports it back the next frame and
    /// the shell sets its own again. Nothing is sent while the title is already right.
    fn sync_window_title(&mut self, ctx: &egui::Context) {
        let doc = self.active_document();
        let want = window_title(doc.as_ref().map(|(name, dirty)| (name.as_str(), *dirty)));
        let reported = ctx.input(|i| i.viewport().title.clone());
        let overridden = reported.is_some_and(|t| t != want);
        if overridden || self.last_title.as_ref() != Some(&want) {
            self.last_title = Some(want.clone());
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(want));
        }
    }

    /// A close request while an editor that is not on screen has unsaved documents: the request
    /// is taken out of the input (eframe reads it after this hook, epi_integration.rs L284, so
    /// nothing closes) and the exit confirm asks. The active editor asks about its own documents
    /// itself. After "Quit anyway" one request passes through untouched.
    fn intercept_close(&mut self, raw: &mut egui::RawInput) {
        let id = raw.viewport_id;
        let Some(info) = raw.viewports.get_mut(&id) else {
            return;
        };
        if !info.close_requested() {
            return;
        }
        if std::mem::take(&mut self.exit_approved) {
            return;
        }
        let dirty = self.dirty_counts();
        if dirty.iter().all(|(mode, _)| *mode == self.mode) {
            return;
        }
        info.events.retain(|e| *e != egui::ViewportEvent::Close);
        self.exit_prompt = Some(dirty);
    }

    /// Help > About: the lockup, the version and the upstream attribution.
    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        // The pinned lockup (brand/SOURCES.json), decoded with the Photo editor's codecs so no
        // image loader has to be installed on the context.
        if self.about_lockup.is_none() {
            let decoded = redrob_photo_codecs::decode(ABOUT_LOCKUP_PNG)
                .ok()
                .map(|img| {
                    let size = [img.width() as usize, img.height() as usize];
                    egui::ColorImage::from_rgba_unmultiplied(size, &img.to_rgba8())
                });
            if let Some(image) = decoded {
                self.about_lockup = Some(ctx.load_texture(
                    "redrob_canvas_about_lockup",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        let mut open = self.about_open;
        egui::Window::new(tf("About {app}", &[("app", services::APP_NAME)]))
            .id(egui::Id::new("redrob_canvas_about"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(12.0);
                    match &self.about_lockup {
                        Some(texture) => {
                            let size = texture.size_vec2();
                            let shown = egui::vec2(
                                size.x * ABOUT_LOCKUP_HEIGHT / size.y,
                                ABOUT_LOCKUP_HEIGHT,
                            );
                            ui.add(
                                egui::Image::from_texture((texture.id(), size))
                                    .fit_to_exact_size(shown),
                            );
                        }
                        None => {
                            ui.label(egui::RichText::new(services::APP_NAME).heading());
                        }
                    }
                    ui.add_space(8.0);
                    ui.label(tf(
                        "Version {version}",
                        &[("version", env!("CARGO_PKG_VERSION"))],
                    ));
                    ui.add_space(8.0);
                    ui.label(t(ABOUT_ATTRIBUTION));
                    ui.add_space(12.0);
                });
            });
        self.about_open = open;
    }

    /// The exit confirm (a modal): the unsaved counts per mode, Cancel, Go to <mode>, Quit anyway.
    fn exit_dialog(&mut self, ctx: &egui::Context) {
        let Some(dirty) = self.exit_prompt.clone() else {
            return;
        };
        let active = self.mode;
        let id = egui::Id::new("redrob_canvas_exit_confirm");
        let modal = egui::Modal::new(id).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading(t("Unsaved changes"));
            for (mode, n) in &dirty {
                let template = if *n == 1 {
                    "{mode}: {n} document with unsaved changes"
                } else {
                    "{mode}: {n} documents with unsaved changes"
                };
                ui.label(tf(
                    template,
                    &[("mode", mode.name()), ("n", &n.to_string())],
                ));
            }
            ui.label(t(
                "Go to a mode to save its documents, or quit and lose them.",
            ));
            ui.add_space(8.0);
            let mut choice = ExitChoice::None;
            ui.horizontal(|ui| {
                if ui.button(t("Cancel")).clicked() {
                    choice = ExitChoice::Cancel;
                }
                for (mode, _) in dirty.iter().filter(|(mode, _)| *mode != active) {
                    if ui
                        .button(tf("Go to {mode}", &[("mode", mode.name())]))
                        .clicked()
                    {
                        choice = ExitChoice::GoTo(*mode);
                    }
                }
                if ui.button(t("Quit anyway")).clicked() {
                    choice = ExitChoice::Quit;
                }
            });
            choice
        });
        let choice = if modal.should_close() {
            ExitChoice::Cancel
        } else {
            modal.inner
        };
        match choice {
            ExitChoice::None => {}
            ExitChoice::Cancel => self.exit_prompt = None,
            ExitChoice::GoTo(mode) => {
                self.exit_prompt = None;
                self.set_mode(mode);
            }
            ExitChoice::Quit => {
                self.exit_prompt = None;
                self.exit_approved = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// The exit confirm is up (for tests).
    #[cfg_attr(not(test), allow(dead_code))] // exercised by the unit tests
    pub fn exit_prompt(&self) -> Option<&[(Mode, usize)]> {
        self.exit_prompt.as_deref()
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

    /// Ctrl+1/2/3 switch modes; Ctrl+Shift+O (and Ctrl+O while the shell owns the keyboard) is
    /// the shell's Open. Consumed before the active editor sees the frame's input.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        for mode in Mode::ALL {
            if ctx.input_mut(|i| i.consume_shortcut(&mode.shortcut())) {
                self.set_mode(mode);
            }
        }
        let open_any = ctx.input_mut(|i| i.consume_shortcut(&OPEN_ANY_SHORTCUT));
        let open = self.shell_owns_focus() && ctx.input_mut(|i| i.consume_shortcut(&OPEN_SHORTCUT));
        if open_any || open {
            self.open_dialog();
        }
    }

    fn ensure_photo(&mut self) -> &mut PhotocraftApp {
        if self.photo.is_none() {
            self.created.push(Mode::Photo);
        }
        let wgpu = self.wgpu.clone();
        self.photo
            .get_or_insert_with(|| timed("build Photo", || Self::build_photo(wgpu)))
    }

    fn build_photo(wgpu: Option<eframe::egui_wgpu::RenderState>) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(
            redrob_photo_engine::Session::new(),
            services::photo::services(),
        );
        // File › Open Location fetches through the shell's HTTP client.
        app.session.fetch_url = Some(Box::new(services::fetch_url));
        // Long commands and file opens run as background jobs with progress and Cancel.
        app.background_jobs = std::env::var_os("REDROB_PHOTO_INLINE_JOBS")
            .or_else(|| std::env::var_os("PHOTOCRAFT_INLINE_JOBS"))
            .is_none(); // legacy: the upstream variable is read when ours is unset
        if let Some(rs) = wgpu
            && rs.adapter.get_info().device_type != eframe::wgpu::DeviceType::Cpu
            && std::env::var_os("REDROB_PHOTO_CPU_CANVAS")
                // legacy: the upstream variable is read when ours is unset
                .or_else(|| std::env::var_os("PHOTOCRAFT_CPU_CANVAS"))
                .is_none()
        // legacy: the upstream variable is read when ours is unset
        {
            app.set_wgpu(rs);
        }
        app
    }

    fn ensure_vector(&mut self) -> &mut VectorcraftApp {
        if self.vector.is_none() {
            self.created.push(Mode::Vector);
        }
        self.vector
            .get_or_insert_with(|| timed("build Vector", Self::build_vector))
    }

    /// The editor `mode` draws with the context the shell prepared: created if need be, and told
    /// once that the host owns the fonts and style.
    fn adopt_context(&mut self, mode: Mode, ctx: &egui::Context) {
        if !self.context_ready || self.adopted.contains(&mode) {
            return;
        }
        match mode {
            Mode::Photo => self.ensure_photo().context_ready(),
            Mode::Vector => self.ensure_vector().adopt_host_context(ctx),
            Mode::Layout => self.ensure_layout().adopt_host_context(ctx),
        }
        self.adopted.push(mode);
    }

    fn build_vector() -> VectorcraftApp {
        services::vector::install_fonts();
        let mut app = VectorcraftApp::new(
            redrob_vector_engine::Session::new(),
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
        self.layout
            .get_or_insert_with(|| timed("build Layout", Self::build_layout))
    }

    fn build_layout() -> DesignApp {
        let mut session = redrob_layout_engine::Session::new();
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
        // The area is never left to the window's clear colour (black): the panel fill is under
        // whatever the editor draws, including on the frame it is created.
        ui.painter().rect_filled(rect, 0.0, ui.visuals().panel_fill);
        self.adopt_context(self.mode(), ui.ctx());
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

/// Run `f`, tracing how long it took.
fn timed<R>(what: &str, f: impl FnOnce() -> R) -> R {
    let t = Instant::now();
    let r = f();
    trace::log(|| format!("{what}: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0));
    r
}

impl eframe::App for ShellApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        i18n::set_current(self.language.effective());
        self.sync_language();
        let t_logic = Instant::now();
        self.frame_count += 1;
        if !self.trace_switches.is_empty() {
            ctx.request_repaint();
            if self.frame_count.is_multiple_of(40) {
                if let Some(next) = self.trace_switches.pop() {
                    self.set_mode(next);
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        } else if self.frame_count == 200
            && std::env::var_os("REDROB_CANVAS_TRACE_SWITCH").is_some()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.shift_held = ctx.input(|i| i.modifiers.shift);
        self.handle_shortcuts(ctx);
        self.flush_pending_opens();
        self.sync_fonts(ctx);
        self.apply_pending(ctx);
        // The agent's tool calls, against the mode the window shows now.
        self.serve_bridge();
        if self.pending.is_some() {
            // The switch waits for the merged fonts: keep the frames coming.
            ctx.request_repaint();
        }
        // Only the active editor runs its frame logic (and so sees this frame's input).
        self.adopt_context(self.mode, ctx);
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
        // An editor created this frame draws in the shell's language from its first frame.
        self.sync_language();
        self.service_ai(ctx);
        if self.frames_since_switch < 12 {
            let n = self.frames_since_switch;
            trace::log(|| {
                let (fonts, families) = ctx.fonts(|f| {
                    (
                        f.definitions().font_data.len(),
                        f.definitions().families.len(),
                    )
                });
                format!(
                    "frame {n} logic: {:.1} ms (live fonts: {fonts} fonts, {families} families)",
                    t_logic.elapsed().as_secs_f64() * 1000.0
                )
            });
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // The shell's own look at the input first: a close while an off-screen editor has unsaved
        // work, and files dropped for another mode, never reach the active editor.
        self.intercept_close(raw);
        self.route_dropped_files(raw);
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
        let t_ui = Instant::now();
        self.sync_window_title(ui.ctx());
        self.top_bar(ui);
        let actions = self.ai_dock(ui);
        for action in actions {
            self.run_ai_action(action);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.editor_area(ui, frame));
        self.about_window(ui.ctx());
        self.exit_dialog(ui.ctx());
        if self.frames_since_switch < 12 {
            let n = self.frames_since_switch;
            trace::log(|| {
                format!(
                    "frame {n} ui: {:.1} ms ({:.1} ms after the ask)",
                    t_ui.elapsed().as_secs_f64() * 1000.0,
                    self.switch_asked
                        .map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0)
                )
            });
            self.frames_since_switch += 1;
            ui.ctx().request_repaint();
        }
    }

    fn on_exit(&mut self) {
        log::info!(
            "closing in {} mode (pending {:?}); editors created: {:?}",
            self.mode().label(),
            self.pending_mode(),
            self.created()
        );
        self.stop_bridge();
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::Arc;

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
    fn a_switch_applies_at_once_and_waits_only_for_a_photo_font_reinstall() {
        let ctx = egui::Context::default();
        let mut shell = ShellApp::new(None);
        let named = |name: &str| {
            let mut defs = egui::FontDefinitions::default();
            defs.families
                .insert(egui::FontFamily::Name(name.into()), vec!["Hack".into()]);
            defs
        };
        let frame = |shell: &mut ShellApp, f: &dyn Fn(&egui::Context)| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                f(ui.ctx());
                shell.sync_fonts(ui.ctx());
                shell.apply_pending(ui.ctx());
            });
            out.textures_delta.clear();
        };
        // The shell's one font set: Photo's definitions with the other editors' families.
        shell.extra_fonts = vec![named("vector-ui"), named("layout-ui")];
        let mut all = named("photo-ui");
        for extra in &shell.extra_fonts {
            merge_extra_fonts(&mut all, extra);
        }
        ctx.set_fonts(all);
        shell.context_ready = true;
        frame(&mut shell, &|_| {});
        assert!(shell.fonts_settled(&ctx));
        // A switch goes through in the frame it is asked for.
        shell.set_mode(Mode::Vector);
        frame(&mut shell, &|_| {});
        assert_eq!((shell.mode(), shell.pending_mode()), (Mode::Vector, None));
        // The Photo editor re-installs its own fonts (UI font size change), dropping the others'
        // families: a switch asked for while that set is live waits one pass...
        frame(&mut shell, &|ctx| ctx.set_fonts(named("photo-ui")));
        shell.set_mode(Mode::Layout);
        frame(&mut shell, &|_| {});
        assert_eq!(
            (shell.mode(), shell.pending_mode()),
            (Mode::Vector, Some(Mode::Layout))
        );
        // ...for the shell to merge them back, once.
        frame(&mut shell, &|_| {});
        let live = ctx.fonts(|f| f.definitions().clone());
        for family in ["photo-ui", "vector-ui", "layout-ui"] {
            assert!(
                live.families
                    .contains_key(&egui::FontFamily::Name(family.into())),
                "{family} is live"
            );
        }
        assert_eq!((shell.mode(), shell.pending_mode()), (Mode::Layout, None));
        assert!(shell.fonts_settled(&ctx));
    }

    #[test]
    fn extra_families_never_replace_the_generic_ones_or_a_shared_name() {
        let mut into = egui::FontDefinitions::default();
        into.families
            .insert(egui::FontFamily::Proportional, vec!["A".into()]);
        into.families
            .insert(egui::FontFamily::Name("shared".into()), vec!["A".into()]);
        let mut extra = egui::FontDefinitions::default();
        extra
            .families
            .insert(egui::FontFamily::Proportional, vec!["B".into()]);
        extra
            .families
            .insert(egui::FontFamily::Name("shared".into()), vec!["B".into()]);
        extra
            .families
            .insert(egui::FontFamily::Name("only-b".into()), vec!["B".into()]);
        merge_extra_fonts(&mut into, &extra);
        assert_eq!(
            into.families[&egui::FontFamily::Proportional],
            vec!["A".to_string()]
        );
        assert_eq!(
            into.families[&egui::FontFamily::Name("shared".into())],
            vec!["A".to_string()]
        );
        assert_eq!(
            into.families[&egui::FontFamily::Name("only-b".into())],
            vec!["B".to_string()]
        );
        assert_eq!(
            into.font_data.len(),
            egui::FontDefinitions::default().font_data.len()
        );
        assert!(has_families(&into, &[extra.clone()]));
        assert!(!has_families(&egui::FontDefinitions::default(), &[extra]));
    }

    #[test]
    fn every_editors_families_are_in_the_shells_font_set() {
        let mut all = egui::FontDefinitions::default();
        let extra = extra_fonts();
        for defs in &extra {
            merge_extra_fonts(&mut all, defs);
        }
        for family in ["ui", "ui-semibold", "mono", "semibold", "arabic"] {
            assert!(
                all.families
                    .contains_key(&egui::FontFamily::Name(family.into())),
                "{family}"
            );
        }
        assert!(has_families(&all, &extra));
    }

    #[test]
    fn the_window_title_names_the_active_document_and_marks_unsaved_changes() {
        assert_eq!(window_title(None), "Redrob Canvas");
        assert_eq!(
            window_title(Some(("poster.psd", false))),
            "poster.psd - Redrob Canvas"
        );
        assert_eq!(
            window_title(Some(("Untitled-1", true))),
            "Untitled-1* - Redrob Canvas"
        );
        let shell = ShellApp::new(None);
        assert_eq!(shell.active_document(), None, "no editor, no document");
        assert!(shell.dirty_counts().is_empty());
    }

    #[test]
    fn the_status_line_fades_after_its_lifetime() {
        let status = Status::new("done", false);
        let now = Instant::now();
        assert_eq!(status.alpha(now), Some(1.0));
        let late = now + Duration::from_secs_f32(STATUS_SECS - 0.5);
        let alpha = status.alpha(late).unwrap_or(0.0);
        assert!(alpha > 0.0 && alpha < 1.0, "{alpha}");
        assert_eq!(status.alpha(now + Duration::from_secs(60)), None);
        let mut with_button = Status::new("opened", false);
        with_button.action = Some(StatusAction::OpenIn(Mode::Layout, PathBuf::from("a.pdf")));
        assert_eq!(
            with_button.alpha(now + Duration::from_secs_f32(STATUS_SECS + 1.0)),
            Some(1.0),
            "a line with a button stays longer"
        );
    }

    #[test]
    fn send_to_enablement_follows_the_handoff_matrix() {
        // Photo -> Layout and Vector exist, Photo -> Photo does not; the menu enables a hand-off
        // only with a document.
        for source in Mode::ALL {
            for target in Mode::ALL {
                let expected = source != target;
                assert_eq!(
                    Handoff::between(source, target).is_some(),
                    expected,
                    "{source:?} -> {target:?}"
                );
            }
        }
        let mut shell = ShellApp::new(None);
        // Sending a mode to itself is refused before any editor is created, and reported.
        shell.send_to(Mode::Photo);
        let status = shell.status().expect("a status line");
        assert!(status.error, "{}", status.text);
        assert!(status.text.starts_with("Send to Photo"), "{}", status.text);
        assert!(status.text.contains("nothing to send"), "{}", status.text);
        assert!(shell.created().is_empty());
    }

    #[test]
    fn dropped_files_for_another_mode_are_taken_from_the_editors_input() {
        #[derive(Debug)]
        struct Dropped(PathBuf);
        impl egui::DroppedFile for Dropped {
            fn path(&self) -> &Path {
                &self.0
            }
            fn bytes(&self) -> Result<Vec<u8>, String> {
                Ok(Vec::new())
            }
        }
        fn dropped(name: &str) -> egui::DroppedFileHandle {
            Arc::new(Dropped(PathBuf::from(name)))
        }
        let mut shell = ShellApp::new(None);
        let mut raw = egui::RawInput {
            dropped_files: vec![
                dropped("a.png"),
                dropped("b.svg"),
                dropped("c.idml"),
                dropped("d.zip"),
            ],
            ..Default::default()
        };
        shell.route_dropped_files(&mut raw);
        // Photo is active: its own PNG and the unknown ZIP stay; SVG and IDML are routed.
        let kept: Vec<String> = raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_string_lossy().to_string())
            .collect();
        assert_eq!(kept, vec!["a.png".to_string(), "d.zip".to_string()]);
        let routed: Vec<(String, Mode)> = shell
            .pending_opens
            .iter()
            .map(|(p, r)| (p.to_string_lossy().to_string(), r.mode))
            .collect();
        assert_eq!(
            routed,
            vec![
                ("b.svg".to_string(), Mode::Vector),
                ("c.idml".to_string(), Mode::Layout)
            ]
        );
        // With Shift held, everything stays for the editor.
        let mut shell = ShellApp::new(None);
        let mut raw = egui::RawInput::default();
        shell.shift_held = true;
        raw.dropped_files = vec![dropped("b.svg")];
        shell.route_dropped_files(&mut raw);
        assert_eq!(raw.dropped_files.len(), 1);
        assert!(shell.pending_opens.is_empty());
    }

    #[test]
    fn a_close_passes_through_while_no_off_screen_editor_is_dirty() {
        let mut shell = ShellApp::new(None);
        let mut raw = egui::RawInput::default();
        raw.viewports
            .entry(raw.viewport_id)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        shell.intercept_close(&mut raw);
        assert!(
            raw.viewport().close_requested(),
            "nothing dirty: the close stands"
        );
        assert_eq!(shell.exit_prompt(), None);
        // The shell-level Open shortcuts: Ctrl+Shift+O always, Ctrl+O only while the shell owns
        // the keyboard (here: no document open).
        assert_eq!(OPEN_SHORTCUT.logical_key, egui::Key::O);
        const { assert!(OPEN_ANY_SHORTCUT.modifiers.shift && OPEN_ANY_SHORTCUT.modifiers.command) };
        assert!(shell.shell_owns_focus());
    }

    #[test]
    fn the_shell_language_is_pushed_into_every_editor_and_adopted_back() {
        let ko = i18n::by_code("ko").unwrap();
        let mut shell = ShellApp::new(None);
        shell.language = i18n::Language::new("ko".into(), i18n::english());
        shell.ensure_vector();
        shell.ensure_layout();
        shell.sync_language();
        assert_eq!(shell.editor_language(Mode::Vector).as_deref(), Some("ko"));
        assert_eq!(shell.editor_language(Mode::Layout).as_deref(), Some("ko"));
        assert_eq!(shell.vector.as_ref().unwrap().ui_language().code(), "ko");
        assert_eq!(shell.language().code, ko.code);
        // The Layout editor's own Interface Language menu picks Japanese: the shell adopts it and
        // the Vector editor follows.
        shell.layout.as_mut().unwrap().ui.language = "ja".into();
        shell.sync_language();
        assert_eq!(shell.language().code, "ja");
        assert_eq!(shell.editor_language(Mode::Vector).as_deref(), Some("ja"));
        // A language not every editor has stays the editor's own; the shell keeps its language.
        shell
            .vector
            .as_mut()
            .unwrap()
            .run(
                "prefs.set",
                json!({ "key": "interfaceLanguage", "value": "ru" }),
            )
            .unwrap();
        shell.sync_language();
        assert_eq!(shell.language().code, "ja");
        assert_eq!(shell.editor_language(Mode::Vector).as_deref(), Some("ru"));
        assert_eq!(shell.editor_language(Mode::Layout).as_deref(), Some("ja"));
        // Back to the system (English here) from the shell's menu: both follow.
        shell.set_language(i18n::AUTO);
        shell.sync_language();
        assert_eq!(shell.editor_language(Mode::Vector).as_deref(), Some("en"));
        assert_eq!(shell.editor_language(Mode::Layout).as_deref(), Some(""));
        assert_eq!(Mode::Vector.name(), "Vector");
        i18n::set_current(ko);
        assert_eq!(Mode::Vector.name(), "벡터");
        i18n::set_current(i18n::english());
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
