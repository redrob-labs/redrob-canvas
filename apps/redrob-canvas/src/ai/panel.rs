// SPDX-License-Identifier: MIT OR Apache-2.0

//! Drawing the AI dock (Tools and Agent tabs) and the top bar's AI menu. Pure egui over
//! [`AiPanel`]: nothing here touches a document; every click becomes an [`Action`] the shell
//! applies after the frame's drawing.

use egui::RichText;
use redrob_ai::iopaint::{Device, EngineState, Plugin, models};

use super::bridge::Capabilities;
use super::jobs::ToolKind;
use super::{Action, AiPanel, Speaker, Tab};
use crate::i18n::t;
use crate::shell::Mode;

/// The dock's default width in points.
pub const DEFAULT_WIDTH: f32 = 320.0;
pub const MIN_WIDTH: f32 = 240.0;
pub const MAX_WIDTH: f32 = 640.0;

/// The top bar's "AI" menu: every tool, disabled when the mode cannot serve it or a job runs,
/// plus the dock toggle. Clicks are queued on the panel.
pub fn menu(ui: &mut egui::Ui, panel: &mut AiPanel, caps: &Capabilities) {
    ui.menu_button("AI", |ui| {
        let busy = panel.is_busy();
        for kind in ToolKind::ALL {
            let enabled = kind.available(caps) && !busy;
            let button = ui.add_enabled(enabled, egui::Button::new(kind.label()));
            let button = match kind.unavailable_reason(caps) {
                Some(reason) if !enabled => button.on_disabled_hover_text(reason),
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
    let text = if panel.is_busy() { "AI ◌" } else { "AI" };
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
                    if ui.small_button("Cancel").clicked() {
                        actions.push(Action::CancelJob);
                    }
                });
            }
            ui.add_space(4.0);
            for kind in ToolKind::ALL {
                card(ui, panel, caps, kind, actions);
            }
            ui.add_space(6.0);
            egui::CollapsingHeader::new("Log")
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

fn engine_row(ui: &mut egui::Ui, panel: &mut AiPanel, actions: &mut Vec<Action>) {
    let state = panel.engine_state();
    let state_text = match &state {
        EngineState::Error(reason) => format!("error: {reason}"),
        other => other.as_str().to_owned(),
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new("Engine").strong());
        let color = match state {
            EngineState::Ready => ui.visuals().selection.stroke.color,
            EngineState::Error(_) => ui.visuals().error_fg_color,
            _ => ui.visuals().weak_text_color(),
        };
        ui.label(RichText::new(state_text).color(color));
    });
    ui.label(RichText::new(panel.engine_message()).small().weak());
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
            "Install"
        } else {
            "Reinstall"
        };
        if ui
            .add_enabled(installable, egui::Button::new(install_label))
            .on_hover_text("IOPaint 1.6.0 and PyTorch into the app's data folder (about 1.5 GB)")
            .clicked()
        {
            actions.push(Action::Install);
        }
        if ui
            .add_enabled(startable, egui::Button::new("Start"))
            .clicked()
        {
            actions.push(Action::Start);
        }
        if ui
            .add_enabled(stoppable, egui::Button::new("Stop"))
            .clicked()
        {
            actions.push(Action::Stop);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Model");
        let mut choices: Vec<String> = models::ERASE_MODELS
            .iter()
            .chain(models::DIFFUSION_MODELS.iter())
            .map(|m| (*m).to_owned())
            .collect();
        if let Some(config) = panel.engine().and_then(|e| e.config()) {
            for downloaded in config.downloaded_models() {
                if !choices.contains(&downloaded) {
                    choices.push(downloaded);
                }
            }
        }
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
    if panel.settings.model != panel.shown_model {
        actions.push(Action::SetModel);
    }
    ui.horizontal(|ui| {
        ui.label("Device");
        for device in Device::ALL {
            if device == Device::Mps && !cfg!(target_os = "macos") {
                continue;
            }
            ui.selectable_value(&mut panel.settings.device, device, device.as_str());
        }
    });
    egui::CollapsingHeader::new("Plugins (loaded at start)")
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

/// One tool card: its parameters, a Run button and the reason it is disabled.
fn card(
    ui: &mut egui::Ui,
    panel: &mut AiPanel,
    caps: &Capabilities,
    kind: ToolKind,
    actions: &mut Vec<Action>,
) {
    let available = kind.available(caps);
    let frame = egui::Frame::group(ui.style());
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(kind.label()).strong());
        let settings = &mut panel.settings;
        match kind {
            ToolKind::Erase => {
                ui.label(
                    RichText::new("Fills the selection from its surroundings (erase model).")
                        .small(),
                );
            }
            ToolKind::Inpaint => {
                ui.add(
                    egui::TextEdit::multiline(&mut settings.prompt)
                        .hint_text("Prompt")
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut settings.negative_prompt)
                        .hint_text("Negative prompt")
                        .desired_width(f32::INFINITY),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.label("Steps");
                    ui.add(egui::DragValue::new(&mut settings.steps).range(1..=150));
                    ui.label("Strength");
                    ui.add(
                        egui::DragValue::new(&mut settings.strength)
                            .range(0.0..=1.0)
                            .speed(0.01),
                    );
                    ui.label("Guidance");
                    ui.add(
                        egui::DragValue::new(&mut settings.guidance)
                            .range(0.0..=30.0)
                            .speed(0.1),
                    );
                    ui.label("Seed");
                    ui.add(egui::DragValue::new(&mut settings.seed).range(-1..=i64::MAX));
                });
                if !models::is_diffusion_model(&settings.model) {
                    ui.label(
                        RichText::new("Pick a diffusion model above to paint from a prompt.")
                            .small()
                            .color(ui.visuals().warn_fg_color),
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
                        ui.label(label);
                        ui.add(
                            egui::DragValue::new(value)
                                .range(0..=redrob_ai::pixels::MAX_EXPAND)
                                .suffix(" px"),
                        );
                    }
                });
                ui.label(RichText::new("Uses the Inpaint prompt when one is set.").small());
            }
            ToolKind::RemoveBackground => {
                ui.label(
                    RichText::new("The subject on a new transparent layer (RemoveBG).").small(),
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
                    ui.label("Preset");
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
                    ui.label(RichText::new("The paths open in Vector mode.").small());
                }
            }
        }
        let busy = panel.is_busy();
        let needs_engine = kind.needs_engine() && panel.engine_state() != EngineState::Ready;
        let enabled = available && !busy && !needs_engine;
        let run = ui.add_enabled(enabled, egui::Button::new("Run"));
        let run = if busy {
            run.on_disabled_hover_text("an AI job is running")
        } else if !available {
            run.on_disabled_hover_text(kind.unavailable_reason(caps).unwrap_or("not available"))
        } else if needs_engine {
            run.on_disabled_hover_text("start the AI engine first")
        } else {
            run
        };
        if run.clicked() {
            actions.push(Action::Run(kind));
        }
        if !available && let Some(reason) = kind.unavailable_reason(caps) {
            ui.label(RichText::new(reason).small().weak());
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
                    RichText::new(
                        "Ask the agent to edit the open document. It can see the mode and \
                         document size and run the tools on the Tools tab.",
                    )
                    .weak(),
                );
            }
            for bubble in &panel.agent.transcript {
                bubble_ui(ui, bubble.speaker, &bubble.text);
            }
            if panel.agent.busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("thinking…").weak());
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
            .hint_text("Message the agent (Ctrl+Enter to send)")
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
            .add_enabled(can_send, egui::Button::new("Send"))
            .clicked();
        if send_clicked || (send_shortcut && can_send) {
            actions.push(Action::AgentSend(std::mem::take(&mut panel.agent.input)));
        }
        if ui.button("New chat").clicked() {
            actions.push(Action::AgentReset);
        }
    });
}

fn bubble_ui(ui: &mut egui::Ui, speaker: Speaker, text: &str) {
    let (who, color) = match speaker {
        Speaker::User => ("You", ui.visuals().strong_text_color()),
        Speaker::Agent => ("Agent", ui.visuals().text_color()),
        Speaker::Tool => ("Tool", ui.visuals().weak_text_color()),
        Speaker::Error => ("Error", ui.visuals().error_fg_color),
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
            .with_size(egui::vec2(360.0, 1400.0))
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
        harness.get_by_label("Engine");
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
