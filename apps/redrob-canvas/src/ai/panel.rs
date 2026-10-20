// SPDX-License-Identifier: MIT OR Apache-2.0

//! Drawing the AI dock (Tools and Agent tabs) and the top bar's AI menu. Pure egui over
//! [`AiPanel`]: nothing here touches a document; every click becomes an [`Action`] the shell
//! applies after the frame's drawing.

use egui::RichText;
use redrob_ai::console::{ImageModel, Operation};
use redrob_ai::iopaint::{Device, EngineState, Plugin, TransportMode, models};

use super::bridge::Capabilities;
use super::jobs::ToolKind;
use super::{Action, AiPanel, Backend, Speaker, Tab, ToolSettings};
use crate::i18n::t;
use crate::shell::Mode;

/// The dock's default width in points.
pub const DEFAULT_WIDTH: f32 = 320.0;
pub const MIN_WIDTH: f32 = 240.0;
pub const MAX_WIDTH: f32 = 640.0;

/// The top bar's "AI" menu: every tool, disabled when the mode cannot serve it or a job runs,
/// plus the dock toggle. Clicks are queued on the panel.
pub fn menu(ui: &mut egui::Ui, panel: &mut AiPanel, caps: &Capabilities) {
    ui.menu_button(t("AI"), |ui| {
        let busy = panel.is_busy();
        for kind in ToolKind::ALL {
            let enabled = kind.available(caps) && !busy;
            let button = ui.add_enabled(enabled, egui::Button::new(t(kind.label())));
            let button = match kind.unavailable_reason(caps) {
                Some(reason) if !enabled => button.on_disabled_hover_text(t(reason)),
                _ if busy => button.on_disabled_hover_text(t("an AI job is running")),
                _ => button,
            };
            if button.clicked() {
                panel.queued.push(Action::Run(kind));
                panel.open = true;
                panel.tab = Tab::Tools;
                ui.close();
            }
        }
        if panel.console.available() {
            for kind in ToolKind::CONSOLE_ONLY {
                let enabled = kind.available(caps) && !busy;
                let button = ui.add_enabled(enabled, egui::Button::new(t(kind.label())));
                let button = match kind.unavailable_reason(caps) {
                    Some(reason) if !enabled => button.on_disabled_hover_text(t(reason)),
                    _ if busy => button.on_disabled_hover_text(t("an AI job is running")),
                    _ => button,
                };
                if button.clicked() {
                    panel.queued.push(Action::Run(kind));
                    panel.open = true;
                    panel.tab = Tab::Tools;
                    ui.close();
                }
            }
        }
        ui.separator();
        let label = if panel.open {
            t("Hide AI panel")
        } else {
            t("Show AI panel")
        };
        if ui.button(label).clicked() {
            panel.open = !panel.open;
            ui.close();
        }
        if ui.button(t("Agent…")).clicked() {
            panel.open = true;
            panel.tab = Tab::Agent;
            ui.close();
        }
    });
}

/// The top bar's right slot: the dock toggle.
pub fn toggle(ui: &mut egui::Ui, panel: &mut AiPanel) {
    let text = if panel.is_busy() {
        format!("{} ◌", t("AI"))
    } else {
        t("AI").to_owned()
    };
    if ui
        .selectable_label(panel.open, text)
        .on_hover_text(t("AI tools and agent (toggle the right panel)"))
        .clicked()
    {
        panel.open = !panel.open;
    }
}

/// The dock's contents. Returns the actions the user asked for this frame.
pub fn dock(ui: &mut egui::Ui, panel: &mut AiPanel, caps: &Capabilities) -> Vec<Action> {
    let mut actions = Vec::new();
    ui.horizontal(|ui| {
        ui.selectable_value(&mut panel.tab, Tab::Tools, t("Tools"));
        ui.selectable_value(&mut panel.tab, Tab::Agent, t("Agent"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("✕").on_hover_text(t("Hide")).clicked() {
                panel.open = false;
            }
        });
    });
    ui.separator();
    match panel.tab {
        Tab::Tools => tools_tab(ui, panel, caps, &mut actions),
        Tab::Agent => agent_tab(ui, panel, &mut actions),
    }
    actions
}

fn tools_tab(
    ui: &mut egui::Ui,
    panel: &mut AiPanel,
    caps: &Capabilities,
    actions: &mut Vec<Action>,
) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            backend_row(ui, panel, actions);
            engine_row(ui, panel, actions);
            ui.add_space(6.0);
            if let Some((text, is_error)) = &panel.status {
                let color = if *is_error {
                    ui.visuals().error_fg_color
                } else {
                    ui.visuals().weak_text_color()
                };
                ui.label(RichText::new(text).color(color));
            }
            if let Some(elapsed) = panel.jobs.elapsed() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("{:.0} s", elapsed.as_secs_f32()));
                    if ui.small_button(t("Cancel")).clicked() {
                        actions.push(Action::CancelJob);
                    }
                });
            }
            ui.add_space(4.0);
            for kind in ToolKind::ALL {
                card(ui, panel, caps, kind, actions);
            }
            if panel.console.available() {
                for kind in ToolKind::CONSOLE_ONLY {
                    card(ui, panel, caps, kind, actions);
                }
            }
            ui.add_space(6.0);
            egui::CollapsingHeader::new(t("Log"))
                .default_open(false)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(160.0)
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for line in &panel.log {
                                ui.label(RichText::new(line).monospace().small());
                            }
                        });
                });
        });
}

/// Where Erase, Generative Fill and Expand run: Redrob Console (online, nothing to install) or
/// this computer (the IOPaint engine, installed once). Always drawn, so the online choice is
/// visible even before the console answers; it is only selectable once the console lists image
/// models (signed in through Redrob Launcher, with the shared engine running).
fn backend_row(ui: &mut egui::Ui, panel: &mut AiPanel, actions: &mut Vec<Action>) {
    let online = panel.console.available();
    ui.label(RichText::new(t("Where AI runs")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(online, |ui| {
            ui.selectable_value(
                &mut panel.settings.backend,
                Backend::Console,
                t("Online (Redrob Console)"),
            )
            .on_hover_text(t(
                "Erase, Generative Fill and Expand run on Redrob's servers. Nothing to install; \
                 usage is billed to your Redrob Console credit.",
            ));
        });
        ui.selectable_value(
            &mut panel.settings.backend,
            Backend::Local,
            t("This computer"),
        )
        .on_hover_text(t(
            "Runs on your computer with the AI engine below. Free, but needs a one-time \
                 install of about 1.5 GB.",
        ));
        let refresh = ui
            .add_enabled(!panel.console.probing(), egui::Button::new("⟳").small())
            .on_hover_text(t("Check Redrob Console again"));
        if refresh.clicked() {
            actions.push(Action::RefreshConsole);
        }
    });
    if !online {
        let text = if panel.console.probing() {
            t("Checking Redrob Console…")
        } else {
            t(
                "To use Online: in Redrob Launcher, sign in and turn the engine on (it comes with \
               Redrob Code). Then press ⟳.",
            )
        };
        ui.label(RichText::new(text).small().weak());
    }
    ui.add_space(4.0);
}

/// The id a console model combo is kept under, one per operation.
fn console_combo_id(op: Operation) -> &'static str {
    match op {
        Operation::Fill => "redrob_canvas_ai_console_fill_model",
        Operation::Expand => "redrob_canvas_ai_console_expand_model",
        Operation::Erase => "redrob_canvas_ai_console_erase_model",
        Operation::Generate => "redrob_canvas_ai_console_generate_model",
    }
}

/// The per-operation model picker over the console's image models.
fn console_model_combo(
    ui: &mut egui::Ui,
    settings: &mut ToolSettings,
    op: Operation,
    choices: &[ImageModel],
) {
    let current = settings.console_model(op).to_owned();
    let shown = choices
        .iter()
        .find(|m| m.id == current)
        .map_or_else(|| short_model(&current), |m| m.display_name().to_owned());
    ui.horizontal(|ui| {
        ui.label(t("Console model"));
        egui::ComboBox::from_id_salt(console_combo_id(op))
            .selected_text(shown)
            .width(ui.available_width().min(200.0))
            .show_ui(ui, |ui| {
                for model in choices {
                    let label = match model.price_per_megapixel_usd {
                        Some(price) => format!("{} · ${price:.3}/MP", model.display_name()),
                        None => model.display_name().to_owned(),
                    };
                    let item = ui.selectable_value(
                        settings.console_model_mut(op),
                        model.id.clone(),
                        label,
                    );
                    if let Some(description) = &model.description {
                        item.on_hover_text(description.as_str());
                    }
                }
            });
    });
}

fn engine_row(ui: &mut egui::Ui, panel: &mut AiPanel, actions: &mut Vec<Action>) {
    let state = panel.engine_state();
    let state_text = match &state {
        EngineState::Missing => t("Not installed").to_owned(),
        EngineState::Installing => t("Installing…").to_owned(),
        EngineState::Stopped => t("Off").to_owned(),
        EngineState::Starting => t("Starting…").to_owned(),
        EngineState::Ready => t("Ready").to_owned(),
        EngineState::Busy => t("Working…").to_owned(),
        EngineState::Error(reason) => format!("{}: {reason}", t("Error")),
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(t("AI engine on this computer")).strong());
        let color = match state {
            EngineState::Ready => ui.visuals().selection.stroke.color,
            EngineState::Error(_) => ui.visuals().error_fg_color,
            _ => ui.visuals().weak_text_color(),
        };
        ui.label(RichText::new(state_text).color(color));
    });
    if state == EngineState::Missing {
        ui.label(
            RichText::new(t(
                "Install once to use the AI tools on this computer (about 1.5 GB). It runs in \
                 the background; no window opens.",
            ))
            .small()
            .weak(),
        );
    } else {
        ui.label(RichText::new(t(&panel.engine_message())).small().weak());
    }
    ui.horizontal_wrapped(|ui| {
        let has_engine = panel.engine().is_some();
        let installable =
            has_engine && matches!(state, EngineState::Missing | EngineState::Stopped);
        let startable = has_engine && matches!(state, EngineState::Stopped | EngineState::Error(_));
        let stoppable = matches!(
            state,
            EngineState::Starting | EngineState::Ready | EngineState::Busy
        );
        let install_label = if state == EngineState::Missing {
            t("Install")
        } else {
            t("Reinstall")
        };
        if ui
            .add_enabled(installable, egui::Button::new(install_label))
            .on_hover_text(t(
                "Downloads IOPaint and PyTorch into the app's data folder (about 1.5 GB)",
            ))
            .clicked()
        {
            actions.push(Action::Install);
        }
        if ui
            .add_enabled(startable, egui::Button::new(t("Start")))
            .clicked()
        {
            actions.push(Action::Start);
        }
        if ui
            .add_enabled(stoppable, egui::Button::new(t("Stop")))
            .clicked()
        {
            actions.push(Action::Stop);
        }
    });
    egui::CollapsingHeader::new(t("Advanced"))
        .id_salt("redrob_canvas_ai_engine_advanced")
        .default_open(false)
        .show(ui, |ui| engine_advanced(ui, panel, actions));
}

/// The engine's model, transport, device and plugins: for people who know IOPaint.
fn engine_advanced(ui: &mut egui::Ui, panel: &mut AiPanel, actions: &mut Vec<Action>) {
    let choices = model_choices(panel, true);
    ui.horizontal(|ui| {
        ui.label(t("Model"));
        egui::ComboBox::from_id_salt("redrob_canvas_ai_model")
            .selected_text(short_model(&panel.settings.model))
            .width(ui.available_width().min(200.0))
            .show_ui(ui, |ui| {
                for choice in choices {
                    let label = short_model(&choice);
                    ui.selectable_value(&mut panel.settings.model, choice, label);
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label(t("Transport"));
        for mode in TransportMode::ALL {
            ui.selectable_value(&mut panel.settings.transport, mode, mode.as_str())
                .on_hover_text(mode.label());
        }
        if let Some(engine) = panel.engine()
            && engine.is_running()
        {
            let in_use = engine.transport_in_use();
            if in_use != panel.settings.transport {
                ui.label(
                    RichText::new(format!("{} {}", t("using"), in_use.as_str()))
                        .small()
                        .weak(),
                )
                .on_hover_text(t(
                    "The running engine has no file routes (started without the shim, or \
                     not on this machine); pixels go as base64 until it is restarted.",
                ));
            }
        }
    });
    if panel.engine_settings_changed() {
        actions.push(Action::SetModel);
    }
    ui.horizontal(|ui| {
        ui.label(t("Device"));
        for device in Device::ALL {
            if device == Device::Mps && !cfg!(target_os = "macos") {
                continue;
            }
            ui.selectable_value(&mut panel.settings.device, device, device.as_str());
        }
    });
    egui::CollapsingHeader::new(t("Plugins (loaded at start)"))
        .default_open(false)
        .show(ui, |ui| {
            for plugin in Plugin::ALL {
                let mut on = panel.settings.plugins.contains(&plugin);
                if ui.checkbox(&mut on, plugin.label()).changed() {
                    if on {
                        panel.settings.plugins.push(plugin);
                    } else {
                        panel.settings.plugins.retain(|p| *p != plugin);
                    }
                }
            }
        });
}

/// `Sanster/PowerPaint-V1-stable-diffusion-inpainting` shows as its last path segment.
pub fn short_model(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).to_owned()
}

/// The models a combo offers: the built-in diffusion list (plus the erase list when
/// `with_erase`), then whatever else the running server has downloaded.
fn model_choices(panel: &AiPanel, with_erase: bool) -> Vec<String> {
    let mut choices: Vec<String> = Vec::new();
    if with_erase {
        choices.extend(models::ERASE_MODELS.iter().map(|m| (*m).to_owned()));
    }
    choices.extend(models::DIFFUSION_MODELS.iter().map(|m| (*m).to_owned()));
    if let Some(config) = panel.engine().and_then(|e| e.config()) {
        for downloaded in config.downloaded_models() {
            let is_erase = models::ERASE_MODELS.contains(&downloaded.as_str());
            if !choices.contains(&downloaded) && (with_erase || !is_erase) {
                choices.push(downloaded);
            }
        }
    }
    choices
}

/// The RemoveBG models a combo offers: the built-in list, then what the running plugin reports.
fn remove_bg_choices(panel: &AiPanel) -> Vec<String> {
    let mut choices: Vec<String> = models::REMOVE_BG_MODELS
        .iter()
        .map(|m| (*m).to_owned())
        .collect();
    if let Some(config) = panel.engine().and_then(|e| e.config())
        && let Some(plugin) = config.plugin_models.get("RemoveBG")
    {
        for choice in &plugin.choices {
            if !choices.contains(choice) {
                choices.push(choice.clone());
            }
        }
    }
    choices
}

/// The "Subject model" combo shared by the Remove background and Select subject cards.
fn remove_bg_model_combo(ui: &mut egui::Ui, panel: &mut AiPanel, id: &str) {
    let choices = remove_bg_choices(panel);
    ui.horizontal(|ui| {
        ui.label(t("Subject model"));
        egui::ComboBox::from_id_salt(id)
            .selected_text(short_model(&panel.settings.remove_bg_model))
            .width(ui.available_width().min(180.0))
            .show_ui(ui, |ui| {
                for choice in choices {
                    let label = short_model(&choice);
                    ui.selectable_value(&mut panel.settings.remove_bg_model, choice, label);
                }
            });
    });
}

/// One tool card: its parameters, a Run button and the reason it is disabled.
fn card(
    ui: &mut egui::Ui,
    panel: &mut AiPanel,
    caps: &Capabilities,
    kind: ToolKind,
    actions: &mut Vec<Action>,
) {
    let available = kind.available(caps);
    let console_op = kind
        .console_operation()
        .filter(|_| panel.settings.routes_to_console(kind));
    let console_models = console_op
        .map(|op| panel.console.models_for(op))
        .unwrap_or_default();
    let fill_models = if kind == ToolKind::GenerativeFill && console_op.is_none() {
        model_choices(panel, false)
    } else {
        Vec::new()
    };
    let frame = egui::Frame::group(ui.style());
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(t(kind.label())).strong());
        if matches!(kind, ToolKind::RemoveBackground | ToolKind::SelectSubject) {
            remove_bg_model_combo(
                ui,
                panel,
                if kind == ToolKind::RemoveBackground {
                    "redrob_canvas_ai_remove_bg_model"
                } else {
                    "redrob_canvas_ai_subject_model"
                },
            );
        }
        let settings = &mut panel.settings;
        match kind {
            ToolKind::Erase => {
                ui.label(
                    RichText::new(t(
                        "Removes what is in the selection and fills it in from its surroundings.",
                    ))
                    .small(),
                );
            }
            ToolKind::Inpaint => {
                ui.add(
                    egui::TextEdit::multiline(&mut settings.prompt)
                        .hint_text(t("Prompt"))
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut settings.negative_prompt)
                        .hint_text(t("Negative prompt"))
                        .desired_width(f32::INFINITY),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.label(t("Steps"));
                    ui.add(egui::DragValue::new(&mut settings.steps).range(1..=150));
                    ui.label(t("Strength"));
                    ui.add(
                        egui::DragValue::new(&mut settings.strength)
                            .range(0.0..=1.0)
                            .speed(0.01),
                    );
                    ui.label(t("Guidance"));
                    ui.add(
                        egui::DragValue::new(&mut settings.guidance)
                            .range(0.0..=30.0)
                            .speed(0.1),
                    );
                    ui.label(t("Seed"));
                    ui.add(egui::DragValue::new(&mut settings.seed).range(-1..=i64::MAX));
                });
                if !models::is_diffusion_model(&settings.model) {
                    ui.label(
                        RichText::new(t(
                            "Pick a diffusion model under Advanced to paint from a prompt.",
                        ))
                        .small()
                        .color(ui.visuals().warn_fg_color),
                    );
                }
            }
            ToolKind::GenerativeFill => {
                ui.add(
                    egui::TextEdit::multiline(&mut settings.fill_prompt)
                        .hint_text(t("What to generate in the selection"))
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut settings.fill_negative_prompt)
                        .hint_text(t("Negative prompt"))
                        .desired_width(f32::INFINITY),
                );
                if console_op.is_none() {
                    ui.horizontal(|ui| {
                        ui.label(t("Fill model"));
                        egui::ComboBox::from_id_salt("redrob_canvas_ai_fill_model")
                            .selected_text(short_model(&settings.fill_model))
                            .width(ui.available_width().min(200.0))
                            .show_ui(ui, |ui| {
                                for choice in fill_models {
                                    let label = short_model(&choice);
                                    ui.selectable_value(&mut settings.fill_model, choice, label);
                                }
                            });
                    });
                    ui.label(
                        RichText::new(t(
                            "Draws what you describe into the selection, on a new layer. The \
                             first run downloads the fill model.",
                        ))
                        .small(),
                    );
                }
            }
            ToolKind::Outpaint => {
                ui.horizontal_wrapped(|ui| {
                    for (label, value) in [
                        ("Left", &mut settings.expand.left),
                        ("Top", &mut settings.expand.top),
                        ("Right", &mut settings.expand.right),
                        ("Bottom", &mut settings.expand.bottom),
                    ] {
                        ui.label(t(label));
                        ui.add(
                            egui::DragValue::new(value)
                                .range(0..=redrob_ai::pixels::MAX_EXPAND)
                                .suffix(" px"),
                        );
                    }
                });
                ui.label(RichText::new(t("Uses the Inpaint prompt when one is set.")).small());
            }
            ToolKind::RemoveBackground => {
                ui.label(
                    RichText::new(t(
                        "Puts the subject on a new layer with a transparent background.",
                    ))
                    .small(),
                );
            }
            ToolKind::Upscale => {
                ui.horizontal(|ui| {
                    ui.label("RealESRGAN");
                    for scale in [2u32, 3, 4] {
                        ui.selectable_value(&mut settings.upscale, scale, format!("x{scale}"));
                    }
                });
            }
            ToolKind::FaceRestore => {
                ui.horizontal(|ui| {
                    for plugin in ["GFPGAN", "RestoreFormer"] {
                        ui.selectable_value(&mut settings.face_plugin, plugin, plugin);
                    }
                });
            }
            ToolKind::SelectSubject => {
                ui.horizontal(|ui| {
                    for plugin in ["RemoveBG", "AnimeSeg"] {
                        ui.selectable_value(&mut settings.subject_plugin, plugin, plugin);
                    }
                });
            }
            ToolKind::Vectorize => {
                ui.horizontal(|ui| {
                    ui.label(t("Preset"));
                    egui::ComboBox::from_id_salt("redrob_canvas_ai_preset")
                        .selected_text(settings.preset.clone())
                        .show_ui(ui, |ui| {
                            for name in redrob_ai::vectorize::preset_names() {
                                let value = (*name).to_owned();
                                ui.selectable_value(&mut settings.preset, value, *name);
                            }
                        });
                });
                if caps.mode != Mode::Vector {
                    ui.label(RichText::new(t("The paths open in Vector mode.")).small());
                }
            }
            ToolKind::Generate => {
                ui.add(
                    egui::TextEdit::multiline(&mut settings.generate_prompt)
                        .hint_text(t("Describe the image to generate"))
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                ui.label(
                    RichText::new(t(
                        "A new layer the size of the document, from the prompt (Redrob Console).",
                    ))
                    .small(),
                );
            }
        }
        if let Some(op) = console_op {
            console_model_combo(ui, &mut panel.settings, op, &console_models);
            if kind != ToolKind::Generate {
                ui.label(
                    RichText::new(t("Runs on Redrob Console: no local engine needed."))
                        .small()
                        .weak(),
                );
            }
        }
        let busy = panel.is_busy();
        let needs_engine = kind.needs_engine()
            && console_op.is_none()
            && panel.engine_state() != EngineState::Ready;
        let enabled = available && !busy && !needs_engine;
        let run = ui.add_enabled(enabled, egui::Button::new(t("Run")));
        let run = if busy {
            run.on_disabled_hover_text(t("an AI job is running"))
        } else if !available {
            run.on_disabled_hover_text(t(kind.unavailable_reason(caps).unwrap_or("not available")))
        } else if needs_engine {
            run.on_disabled_hover_text(t("install and start the AI engine first, or choose Online"))
        } else {
            run
        };
        if run.clicked() {
            actions.push(Action::Run(kind));
        }
        if !available && let Some(reason) = kind.unavailable_reason(caps) {
            ui.label(RichText::new(t(reason)).small().weak());
        }
    });
}

fn agent_tab(ui: &mut egui::Ui, panel: &mut AiPanel, actions: &mut Vec<Action>) {
    let input_height = 92.0;
    let transcript_height = (ui.available_height() - input_height).max(60.0);
    egui::ScrollArea::vertical()
        .id_salt("redrob_canvas_ai_transcript")
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .max_height(transcript_height)
        .show(ui, |ui| {
            if panel.agent.transcript.is_empty() {
                ui.label(
                    RichText::new(t(
                        "Ask the agent to edit the open document. It can see the mode and \
                         document size and run the tools on the Tools tab.",
                    ))
                    .weak(),
                );
            }
            for bubble in &panel.agent.transcript {
                bubble_ui(ui, bubble.speaker, &bubble.text);
            }
            if panel.agent.busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(t("thinking…")).weak());
                });
            }
        });
    ui.separator();
    if let Some(error) = &panel.agent.error {
        ui.label(
            RichText::new(error)
                .color(ui.visuals().error_fg_color)
                .small(),
        );
    }
    let response = ui.add(
        egui::TextEdit::multiline(&mut panel.agent.input)
            .hint_text(t("Message the agent (Ctrl+Enter to send)"))
            .desired_rows(2)
            .desired_width(f32::INFINITY),
    );
    let send_shortcut = response.has_focus()
        && ui.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::Enter,
            ))
        });
    ui.horizontal(|ui| {
        let can_send = !panel.agent.busy && !panel.agent.input.trim().is_empty();
        let send_clicked = ui
            .add_enabled(can_send, egui::Button::new(t("Send")))
            .clicked();
        if send_clicked || (send_shortcut && can_send) {
            actions.push(Action::AgentSend(std::mem::take(&mut panel.agent.input)));
        }
        if ui.button(t("New chat")).clicked() {
            actions.push(Action::AgentReset);
        }
    });
}

fn bubble_ui(ui: &mut egui::Ui, speaker: Speaker, text: &str) {
    let (who, color) = match speaker {
        Speaker::User => (t("You"), ui.visuals().strong_text_color()),
        Speaker::Agent => (t("Agent"), ui.visuals().text_color()),
        Speaker::Tool => (t("Tool"), ui.visuals().weak_text_color()),
        Speaker::Error => (t("Error"), ui.visuals().error_fg_color),
    };
    let fill = match speaker {
        Speaker::User => ui.visuals().faint_bg_color,
        _ => ui.visuals().extreme_bg_color,
    };
    egui::Frame::NONE
        .fill(fill)
        .corner_radius(6.0)
        .inner_margin(6.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(who).small().color(color));
            let body = RichText::new(text).color(color);
            if speaker == Speaker::Tool {
                ui.label(body.monospace().small());
            } else {
                ui.label(body);
            }
        });
    ui.add_space(4.0);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn short_model_names() {
        assert_eq!(short_model("lama"), "lama");
        assert_eq!(
            short_model("runwayml/stable-diffusion-inpainting"),
            "stable-diffusion-inpainting"
        );
    }

    #[test]
    fn the_menu_lists_every_tool_and_disables_what_the_mode_cannot_serve() {
        use egui_kittest::kittest::{NodeT, Queryable};
        struct State {
            panel: AiPanel,
            caps: Capabilities,
        }
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui_state(
                |ui, state: &mut State| {
                    ui.horizontal(|ui| menu(ui, &mut state.panel, &state.caps));
                },
                State {
                    panel: AiPanel::new(None),
                    caps: Capabilities::layout(true),
                },
            );
        harness.run();
        harness.get_by_label("AI").click();
        harness.run();
        for kind in ToolKind::ALL {
            let item = harness.get_by_label(kind.label());
            let expect = kind.available(&Capabilities::layout(true));
            assert_eq!(
                item.accesskit_node().is_disabled(),
                !expect,
                "{kind:?} in Layout"
            );
        }
        assert!(
            harness
                .get_by_label(ToolKind::GenerativeFill.label())
                .accesskit_node()
                .is_disabled(),
            "Generative Fill needs a pixel selection"
        );
        // Clicking an enabled item queues the run and opens the dock on the Tools tab.
        harness.get_by_label(ToolKind::Vectorize.label()).click();
        harness.run();
        assert_eq!(
            harness.state().panel.queued,
            vec![Action::Run(ToolKind::Vectorize)]
        );
        assert!(harness.state().panel.open);
        assert_eq!(harness.state().panel.tab, Tab::Tools);
        // In Photo with a selection everything is enabled.
        harness.state_mut().caps = Capabilities::photo(true, true);
        harness.state_mut().panel.queued.clear();
        harness.run();
        harness.get_by_label("AI").click();
        harness.run();
        for kind in ToolKind::ALL {
            assert!(
                !harness
                    .get_by_label(kind.label())
                    .accesskit_node()
                    .is_disabled(),
                "{kind:?} in Photo"
            );
        }
    }

    #[test]
    fn the_dock_shows_cards_and_the_engine_row() {
        use egui_kittest::kittest::{NodeT, Queryable};
        struct State {
            panel: AiPanel,
            caps: Capabilities,
            actions: Vec<Action>,
        }
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(360.0, 1800.0))
            .build_ui_state(
                |ui, state: &mut State| {
                    let actions = dock(ui, &mut state.panel, &state.caps);
                    state.actions.extend(actions);
                },
                State {
                    panel: AiPanel::new(None),
                    caps: Capabilities::photo(true, false),
                    actions: Vec::new(),
                },
            );
        harness.run();
        harness.get_by_label("AI engine on this computer");
        harness.get_by_label("Advanced");
        harness.get_by_label("Fill model");
        for kind in ToolKind::ALL {
            harness.get_by_label(kind.label());
        }
        // Without an engine every Run but Vectorize's is disabled; Vectorize runs in-process.
        let runs = harness.get_all_by_label("Run").collect::<Vec<_>>();
        assert_eq!(runs.len(), ToolKind::ALL.len());
        let enabled = runs
            .iter()
            .filter(|r| !r.accesskit_node().is_disabled())
            .count();
        assert_eq!(enabled, 1);
        harness.get_by_label("Agent").click();
        harness.run();
        harness.get_by_label("Send");
        assert_eq!(harness.state().panel.tab, Tab::Agent);
        assert!(harness.state().actions.is_empty(), "nothing was run");
    }
}
