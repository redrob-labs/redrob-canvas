// SPDX-License-Identifier: MIT OR Apache-2.0
//! The animation side of the Timeline panel: a frame table per animated layer (keyframes, hold
//! and clone frames, cached frames), keyframe actions, playback range, onion skin, animation
//! curves, audio, storyboard and recorder controls. Every action runs an engine `anim.*` command,
//! so the control channel drives the same thing.

use egui::{Color32, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use redrob_photo_doc::Timeline;
use redrob_photo_engine::anim_cmds;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use redrob_photo_engine::Session;

use crate::theme::Tokens;

/// A file dialog the docker asks for (opened by the caller, which owns the app).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialog {
    AttachAudio,
    ExportStoryboard,
    ExportTimelapse,
}

/// What a frame of the docker asks the app to do.
#[derive(Default)]
pub struct Out {
    pub act: Option<(&'static str, Value)>,
    pub select: Option<redrob_photo_doc::LayerId>,
    pub dialog: Option<Dialog>,
}

/// Which section of the animation docker is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Section {
    #[default]
    Frames,
    OnionSkin,
    Curves,
    Storyboard,
    Recorder,
}

/// Animation docker view state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnimationUi {
    pub section: Section,
    /// Frame cell width in points (zoom).
    pub zoom: f32,
    /// First frame shown in the table.
    pub scroll: usize,
    /// Scene being edited in the storyboard (name / comment text).
    pub scene_comment: String,
    pub scene_name: String,
}

impl Default for AnimationUi {
    fn default() -> Self {
        AnimationUi { section: Section::Frames, zoom: 14.0, scroll: 0, scene_comment: String::new(), scene_name: String::new() }
    }
}

/// Draw the animation sections under the timeline scrubber. Returns the command to run, if any.
pub fn section(s: &Session, anim: &mut AnimationUi, ui: &mut egui::Ui, tl: &Timeline) -> Out {
    let t = Tokens::get(ui.ctx());
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        for (sec, label) in [
            (Section::Frames, tl!("Frames")),
            (Section::OnionSkin, tl!("Onion Skin")),
            (Section::Curves, tl!("Curves")),
            (Section::Storyboard, tl!("Storyboard")),
            (Section::Recorder, tl!("Recorder")),
        ] {
            if ui.selectable_label(anim.section == sec, label).clicked() {
                anim.section = sec;
            }
        }
    });
    ui.separator();
    match anim.section {
        Section::Frames => frames(s, anim, ui, tl, &t),
        Section::OnionSkin => onion(s, ui, &t),
        Section::Curves => curves(s, ui, tl, &t),
        Section::Storyboard => storyboard(s, anim, ui, tl, &t),
        Section::Recorder => recorder(s, ui, &t),
    }
}

fn frames(s: &Session, anim: &mut AnimationUi, ui: &mut egui::Ui, tl: &Timeline, t: &Tokens) -> Out {
    let mut out = Out::default();
    let Some(st) = s.active() else { return out };
    let doc = st.doc.clone();
    let active = st.active_layer;
    let a = anim_cmds::animation(&doc);
    ui.horizontal(|ui| {
        if crate::icons::button(ui, "plus", 22.0, false, tl!("New keyframe")).clicked() {
            out.act = Some(("anim.keyframe.add", json!({})));
        }
        if crate::icons::button(ui, "copy", 22.0, false, tl!("Duplicate keyframe")).clicked() {
            out.act = Some(("anim.keyframe.duplicate", json!({})));
        }
        if crate::icons::button(ui, "trash", 22.0, false, tl!("Remove keyframe")).clicked() {
            out.act = Some(("anim.keyframe.remove", json!({})));
        }
        ui.separator();
        let mut fps = tl.fps;
        let r = crate::widgets::popup_value_field(ui, tl!("Frame rate"), &mut fps, 1.0..=240.0, " fps", 64.0);
        if r.changed {
            out.act = Some(("timeline.setProps", json!({"fps": fps})));
        }
        let mut ws = tl.work_start as f32;
        let mut we = tl.work_end as f32;
        let r1 = crate::widgets::popup_value_field(ui, tl!("Start"), &mut ws, 0.0..=tl.duration as f32, "", 56.0);
        let r2 = crate::widgets::popup_value_field(ui, tl!("End"), &mut we, 1.0..=tl.duration as f32, "", 56.0);
        if r1.changed || r2.changed {
            out.act = Some(("timeline.setProps", json!({"workStart": ws.round() as u64, "workEnd": we.round() as u64})));
        }
        ui.separator();
        if crate::icons::button(ui, "zoom-in", 20.0, false, tl!("Zoom in")).clicked() {
            anim.zoom = (anim.zoom * 1.25).min(48.0);
        }
        if crate::icons::button(ui, "zoom-out", 20.0, false, tl!("Zoom out")).clicked() {
            anim.zoom = (anim.zoom / 1.25).max(6.0);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let audio = a.audio.as_ref();
            let muted = audio.is_some_and(|au| au.muted);
            if let Some(au) = audio {
                if crate::icons::button(ui, if muted { "volume-x" } else { "volume-2" }, 20.0, muted, tl!("Mute audio")).clicked() {
                    out.act = Some(("anim.audio.mute", json!({})));
                }
                ui.label(
                    RichText::new(std::path::Path::new(&au.path).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default())
                        .color(t.text_dim)
                        .small(),
                );
            } else if ui.small_button(tl!("Attach audio…")).clicked() {
                out.dialog = Some(Dialog::AttachAudio);
            }
        });
    });
    ui.add_space(4.0);
    // The frame table.
    let cell = anim.zoom;
    let name_w = 120.0;
    let avail = ui.available_width() - name_w - 8.0;
    let visible = ((avail / cell).floor() as usize).max(1);
    let scroll = anim.scroll.min(tl.duration.saturating_sub(1));
    let end = (scroll + visible).min(tl.duration.max(1));
    let layers: Vec<(redrob_photo_doc::LayerId, String, anim_cmds::Keys, Vec<bool>)> = doc
        .walk()
        .into_iter()
        .filter_map(|(_, _, l)| {
            let keys = anim_cmds::keys_of(l)?;
            let cached: Vec<bool> =
                (scroll..end).map(|f| l.video.as_ref().and_then(|v| v.frames.get(f)).is_some_and(|s| !s.content_bounds().is_empty())).collect();
            Some((l.id, l.name.clone(), keys, cached))
        })
        .collect();
    // Header: frame numbers every 5 frames, the playhead and the work area.
    let (hdr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::click_and_drag());
    let painter = ui.painter_at(hdr);
    let x0 = hdr.left() + name_w;
    let wa = egui::Rect::from_min_max(
        pos2(x0 + (tl.work_start.saturating_sub(scroll)) as f32 * cell, hdr.top()),
        pos2(x0 + (tl.work_end.saturating_sub(scroll)) as f32 * cell, hdr.bottom()),
    );
    painter.rect_filled(wa.intersect(hdr), 0.0, t.accent.gamma_multiply(0.15));
    for f in scroll..end {
        let x = x0 + (f - scroll) as f32 * cell;
        if f % 5 == 0 {
            painter.text(pos2(x + 2.0, hdr.center().y), egui::Align2::LEFT_CENTER, f.to_string(), egui::FontId::proportional(9.0), t.text_dim);
        }
        if f == tl.current {
            painter.rect_filled(egui::Rect::from_min_max(pos2(x, hdr.top()), pos2(x + cell, hdr.bottom())), 2.0, t.accent.gamma_multiply(0.5));
        }
    }
    if let Some(p) = ui.ctx().pointer_interact_pos().filter(|_| ui.rect_contains_pointer(hdr) && ui.input(|i| i.pointer.primary_down())) {
        let f = (((p.x - x0) / cell).floor().max(0.0) as usize + scroll).min(tl.duration.saturating_sub(1));
        if f != tl.current {
            out.act = Some(("timeline.setFrame", json!({"frame": f as u64})));
        }
    }
    if layers.is_empty() {
        ui.weak(tl!("No animated layers: add a keyframe to the active layer to animate it."));
    }
    for (id, name, keys, cached) in &layers {
        let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), cell.max(14.0) + 4.0), Sense::click());
        let painter = ui.painter_at(row);
        let selected = active == Some(*id);
        if selected {
            painter.rect_filled(row, 0.0, t.accent.gamma_multiply(0.12));
        }
        painter.text(pos2(row.left() + 4.0, row.center().y), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(11.0), t.text);
        for f in scroll..end {
            let x = x0 + (f - scroll) as f32 * cell;
            let r = egui::Rect::from_min_max(pos2(x + 1.0, row.top() + 2.0), pos2(x + cell - 1.0, row.bottom() - 2.0));
            let status = keys.status(f);
            let fill = match status {
                "key" => t.text,
                "clone" => t.accent,
                "hold" => t.text_faint,
                _ => Color32::TRANSPARENT,
            };
            if status == "hold" {
                painter.rect_filled(egui::Rect::from_center_size(r.center(), vec2(r.width(), 3.0)), 1.0, fill);
            } else if status != "empty" {
                painter.rect_filled(r, 2.0, fill);
            }
            if cached.get(f - scroll).copied().unwrap_or(false) {
                painter.rect_filled(egui::Rect::from_min_size(pos2(r.left(), row.bottom() - 2.0), vec2(r.width(), 1.5)), 0.0, t.accent.gamma_multiply(0.6));
            }
            if f == tl.current {
                painter.rect_stroke(r, 2.0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
            }
        }
        if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            if p.x >= x0 {
                let f = (((p.x - x0) / cell).floor().max(0.0) as usize + scroll).min(tl.duration.saturating_sub(1));
                out.act = Some(("timeline.setFrame", json!({"frame": f as u64})));
            }
            out.select = Some(*id);
        }
        resp.context_menu(|ui| {
            if ui.button(tl!("New keyframe here")).clicked() {
                out.act = Some(("anim.keyframe.add", json!({"layer": id.0})));
                ui.close();
            }
            if ui.button(tl!("Duplicate keyframe here")).clicked() {
                out.act = Some(("anim.keyframe.duplicate", json!({"layer": id.0})));
                ui.close();
            }
            if ui.button(tl!("Clone previous keyframe here")).clicked() {
                let from = keys.key_of(tl.current.saturating_sub(1)).unwrap_or(0);
                out.act = Some(("anim.keyframe.clone", json!({"layer": id.0, "from": from, "to": tl.current})));
                ui.close();
            }
            if ui.button(tl!("Remove keyframe")).clicked() {
                out.act = Some(("anim.keyframe.remove", json!({"layer": id.0})));
                ui.close();
            }
        });
    }
    // Horizontal scroll of the table with the wheel.
    let wheel = ui.input(|i| i.smooth_scroll_delta.x + if i.modifiers.shift { i.smooth_scroll_delta.y } else { 0.0 });
    if wheel != 0.0 && ui.rect_contains_pointer(hdr.union(ui.min_rect())) {
        let step = (wheel.abs() / cell).ceil() as usize;
        anim.scroll = if wheel > 0.0 { scroll.saturating_sub(step) } else { (scroll + step).min(tl.duration.saturating_sub(1)) };
    }
    out
}

fn onion(s: &Session, ui: &mut egui::Ui, t: &Tokens) -> Out {
    let mut out = Out::default();
    let Some(st) = s.active() else { return out };
    let mut o = anim_cmds::animation(&st.doc).onion;
    let mut changed = false;
    let mut enabled = o.enabled;
    if crate::widgets::checkbox(ui, &mut enabled, tl!("Show onion skins")).changed() {
        o.enabled = enabled;
        changed = true;
    }
    ui.horizontal(|ui| {
        let mut before = o.before as f32;
        let mut after = o.after as f32;
        if crate::widgets::popup_value_field(ui, tl!("Before"), &mut before, 0.0..=10.0, "", 56.0).changed {
            o.before = before.round() as usize;
            changed = true;
        }
        if crate::widgets::popup_value_field(ui, tl!("After"), &mut after, 0.0..=10.0, "", 56.0).changed {
            o.after = after.round() as usize;
            changed = true;
        }
        let mut strength = o.tint_strength;
        if crate::widgets::popup_value_field(ui, tl!("Tint"), &mut strength, 0.0..=100.0, "%", 56.0).changed {
            o.tint_strength = strength;
            changed = true;
        }
    });
    ui.label(RichText::new(tl!("Opacity per frame")).color(t.text_dim).small());
    ui.horizontal(|ui| {
        let n = o.before.max(o.after).max(1);
        while o.opacities.len() < n {
            o.opacities.push(10.0);
        }
        for i in 0..n {
            let mut v = o.opacities[i];
            ui.vertical(|ui| {
                ui.label(RichText::new(format!("±{}", i + 1)).small().color(t.text_dim));
                let r = ui.add(egui::Slider::new(&mut v, 0.0..=100.0).vertical().show_value(false));
                if r.changed() {
                    o.opacities[i] = v;
                    changed = true;
                }
            });
        }
    });
    ui.horizontal(|ui| {
        for (label, slot) in [(tl!("Previous tint"), &mut o.tint_before), (tl!("Next tint"), &mut o.tint_after)] {
            ui.label(label);
            let mut c = Color32::from_rgb((slot[0] * 255.0) as u8, (slot[1] * 255.0) as u8, (slot[2] * 255.0) as u8);
            if ui.color_edit_button_srgba(&mut c).changed() {
                *slot = [f32::from(c.r()) / 255.0, f32::from(c.g()) / 255.0, f32::from(c.b()) / 255.0];
                changed = true;
            }
        }
    });
    if changed {
        out.act = Some((
            "anim.onionSkin.set",
            json!({"enabled": o.enabled, "before": o.before, "after": o.after, "opacities": o.opacities, "tintBefore": o.tint_before, "tintAfter": o.tint_after, "tintStrength": o.tint_strength}),
        ));
    }
    out
}

fn curves(s: &Session, ui: &mut egui::Ui, tl: &Timeline, t: &Tokens) -> Out {
    let mut out = Out::default();
    let Some(st) = s.active() else { return out };
    let doc = st.doc.clone();
    let active = st.active_layer;
    ui.horizontal(|ui| {
        ui.label(tl!("Key the active layer:"));
        for (channel, label) in
            [("opacity", tl!("Opacity")), ("x", "X"), ("y", "Y"), ("scaleX", tl!("Scale X")), ("scaleY", tl!("Scale Y")), ("rotation", tl!("Rotation"))]
        {
            if ui.small_button(label).clicked() {
                let value = match channel {
                    "opacity" => Value::Null,
                    "scaleX" | "scaleY" => json!(1.0),
                    _ => json!(0.0),
                };
                let mut p = json!({"channel": channel});
                if !value.is_null() {
                    p["value"] = value;
                }
                out.act = Some(("anim.curves.addKey", p));
            }
        }
    });
    let mut any = false;
    for (_, _, l) in doc.walk() {
        let Some(c) = redrob_photo_engine::layer_meta::get::<anim_cmds::Curves>(l, anim_cmds::CURVES) else { continue };
        any = true;
        ui.label(RichText::new(&l.name).strong().color(if active == Some(l.id) { t.accent } else { t.text }));
        for ch in &c.channels {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 3.0, t.text.gamma_multiply(0.04));
            painter.text(rect.left_top() + vec2(4.0, 4.0), egui::Align2::LEFT_TOP, tl!(ch.name.as_str()), egui::FontId::proportional(10.0), t.text_dim);
            let n = tl.duration.max(2);
            let values: Vec<f64> = (0..n).map(|f| anim_cmds::channel_at(ch, f).unwrap_or(0.0)).collect();
            let (lo, hi) = values.iter().fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
            let span = (hi - lo).max(1e-6);
            let pts: Vec<egui::Pos2> = values
                .iter()
                .enumerate()
                .map(|(f, v)| {
                    pos2(rect.left() + f as f32 / (n - 1) as f32 * rect.width(), rect.bottom() - 4.0 - ((v - lo) / span) as f32 * (rect.height() - 8.0))
                })
                .collect();
            painter.add(egui::Shape::line(pts.clone(), Stroke::new(1.5, t.accent)));
            for k in &ch.keys {
                if let Some(p) = pts.get(k.frame) {
                    painter.circle_filled(*p, 3.0, t.text);
                    if let Some(h) = k.handle_out {
                        let q = pos2(p.x + h[0] as f32 / (n - 1) as f32 * rect.width(), p.y - (h[1] / span) as f32 * (rect.height() - 8.0));
                        painter.line_segment([*p, q], Stroke::new(1.0, t.text_dim));
                        painter.circle_stroke(q, 2.0, Stroke::new(1.0, t.text_dim));
                    }
                }
            }
            if let Some(p) = pts.get(tl.current) {
                painter.line_segment([pos2(p.x, rect.top()), pos2(p.x, rect.bottom())], Stroke::new(1.0, t.accent.gamma_multiply(0.6)));
            }
        }
    }
    if !any {
        ui.weak(tl!("No animation curves yet."));
    }
    out
}

fn storyboard(s: &Session, anim: &mut AnimationUi, ui: &mut egui::Ui, tl: &Timeline, t: &Tokens) -> Out {
    let mut out = Out::default();
    let Some(st) = s.active() else { return out };
    let scenes = anim_cmds::animation(&st.doc).storyboard;
    ui.horizontal(|ui| {
        let w = 120.0;
        crate::widgets::text_field(ui, ui.id().with("scene-name"), &mut anim.scene_name, tl!("Scene name"), w, false);
        crate::widgets::text_field(ui, ui.id().with("scene-comment"), &mut anim.scene_comment, tl!("Comment"), w * 2.0, false);
        if ui.small_button(tl!("Add scene at playhead")).clicked() {
            let mut p = json!({"frame": tl.current, "comment": anim.scene_comment});
            if !anim.scene_name.trim().is_empty() {
                p["name"] = json!(anim.scene_name.trim());
            }
            out.act = Some(("anim.storyboard.add", p));
            anim.scene_name.clear();
            anim.scene_comment.clear();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button(tl!("Export…")).clicked() {
                out.dialog = Some(Dialog::ExportStoryboard);
            }
        });
    });
    if scenes.is_empty() {
        ui.weak(tl!("No scenes yet."));
    }
    for (i, sc) in scenes.iter().enumerate() {
        ui.horizontal(|ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(48.0, 32.0), Sense::click());
            ui.painter().rect_filled(r, 3.0, t.text.gamma_multiply(0.08));
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, sc.frame.to_string(), egui::FontId::proportional(11.0), t.text);
            if resp.clicked() {
                out.act = Some(("timeline.setFrame", json!({"frame": sc.frame as u64})));
            }
            ui.vertical(|ui| {
                ui.label(RichText::new(format!("{}  ·  {} {}", sc.name, sc.duration, tl!("frames"))).color(t.text));
                if !sc.comment.is_empty() {
                    ui.label(RichText::new(&sc.comment).small().color(t.text_dim));
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::icons::button(ui, "trash", 18.0, false, tl!("Remove scene")).clicked() {
                    out.act = Some(("anim.storyboard.remove", json!({"index": i})));
                }
                let mut d = sc.duration as f32;
                if crate::widgets::popup_value_field(ui, tl!("Duration"), &mut d, 1.0..=10000.0, "", 56.0).changed {
                    out.act = Some(("anim.storyboard.update", json!({"index": i, "duration": d.round() as u64})));
                }
            });
        });
    }
    out
}

fn recorder(s: &Session, ui: &mut egui::Ui, t: &Tokens) -> Out {
    let mut out = Out::default();
    let Some(st) = s.active() else { return out };
    let rec = anim_cmds::animation(&st.doc).recorder;
    ui.horizontal(|ui| {
        if rec.enabled {
            if crate::widgets::primary_button(ui, tl!("Stop recording"), 140.0).clicked() {
                out.act = Some(("anim.recorder.stop", json!({})));
            }
        } else if crate::widgets::primary_button(ui, tl!("Start recording"), 140.0).clicked() {
            out.act = Some(("anim.recorder.start", json!({})));
        }
        if ui.small_button(tl!("Capture now")).clicked() {
            out.act = Some(("anim.recorder.capture", json!({})));
        }
        if ui.add_enabled(rec.count > 0, egui::Button::new(tl!("Export timelapse…")).small()).clicked() {
            out.dialog = Some(Dialog::ExportTimelapse);
        }
    });
    ui.label(
        RichText::new(crate::i18n::fmt(
            tl!("{count} snapshots in {dir}"),
            &[("count", &rec.count.to_string()), ("dir", if rec.dir.is_empty() { "—" } else { rec.dir.as_str() })],
        ))
        .color(t.text_dim)
        .small(),
    );
    out
}
