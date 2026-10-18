// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Layers panel's grouped blend-mode menu: favourites first (a star on each row toggles
//! them), then the modes by family (arithmetic, binary, darken, HSX, lighten, misc, mix, modulo,
//! negative, quadratic) with headers. A typed letter jumps to the next mode starting with it,
//! Return closes; the wheel over the button steps through the flat list as before.

use egui::{RichText, Ui};
use redrob_photo_color::BlendMode;

use crate::theme::Tokens;

/// The default favourites of a fresh profile.
pub const DEFAULT_FAVOURITES: [BlendMode; 4] = [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen, BlendMode::Overlay];

/// The families of the grouped menu, in display order, with their modes.
pub fn families() -> Vec<(&'static str, Vec<BlendMode>)> {
    use BlendMode as B;
    let mut out: Vec<(&'static str, Vec<BlendMode>)> = vec![
        (
            "Mix",
            vec![B::Normal, B::Overlay, B::SoftLight, B::HardLight, B::VividLight, B::LinearLight, B::PinLight, B::HardMix, B::GrainExtract, B::GrainMerge],
        ),
        ("Arithmetic", vec![B::LinearDodge, B::Subtract, B::Divide, B::Multiply]),
        ("Darken", vec![B::Darken, B::ColorBurn, B::LinearBurn, B::DarkerColor, B::PaintNetColorBurn]),
        ("Lighten", vec![B::Lighten, B::Screen, B::ColorDodge, B::LighterColor, B::PaintNetColorDodge]),
        ("Negative", vec![B::Difference, B::Exclusion, B::Negation]),
        (
            "HSX",
            vec![
                B::Hue,
                B::Saturation,
                B::Color,
                B::Luminosity,
                B::HsvHue,
                B::HsvSaturation,
                B::HsvValue,
                B::Luminance,
                B::LchHue,
                B::LchChroma,
                B::LchColor,
                B::LchLightness,
            ],
        ),
        ("Binary", B::BINARY_MODES.iter().copied().chain(std::iter::once(B::Xor)).collect()),
        ("Modulo", B::MODULO_MODES.to_vec()),
        ("Quadratic", std::iter::once(B::Reflect).chain(std::iter::once(B::Glow)).chain(B::QUADRATIC_MODES).collect()),
        ("Misc", vec![B::Dissolve, B::Erase, B::Merge, B::Split]),
    ];
    // Every mode is listed once: anything the table above forgot lands in Misc.
    let listed: Vec<BlendMode> = out.iter().flat_map(|(_, m)| m.iter().copied()).collect();
    for m in BlendMode::layer_modes() {
        if !listed.contains(&m)
            && let Some((_, misc)) = out.iter_mut().find(|(n, _)| *n == "Misc")
        {
            misc.push(m);
        }
    }
    out
}

/// The grouped menu as a flat option list (favourites, then the families in order), for the
/// wheel and for the control channel.
pub fn flat(options: &[(BlendMode, &'static str)], favourites: &[BlendMode]) -> Vec<BlendMode> {
    let available: Vec<BlendMode> = options.iter().map(|(m, _)| *m).collect();
    let mut out: Vec<BlendMode> = favourites.iter().copied().filter(|m| available.contains(m)).collect();
    for (_, modes) in families() {
        for m in modes {
            if available.contains(&m) && !out.contains(&m) {
                out.push(m);
            }
        }
    }
    for m in available {
        if !out.contains(&m) {
            out.push(m);
        }
    }
    out
}

/// Draw the menu. `options` are the modes the layer may take (with labels); `favourites` is the
/// user's list, edited in place by the stars. Returns the modes chosen this frame (one per click,
/// key or wheel notch; the last is `current`) and the mode under the pointer in the open list.
pub fn menu(
    ui: &mut Ui,
    id: &str,
    current: &mut BlendMode,
    options: &[(BlendMode, &'static str)],
    favourites: &mut Vec<BlendMode>,
    width: f32,
) -> (Vec<BlendMode>, Option<BlendMode>) {
    let t = Tokens::get(ui.ctx());
    let label = options.iter().find(|(m, _)| m == current).map(|(_, l)| tl!(l).to_string()).unwrap_or_else(|| "—".into());
    let (mut chosen, mut hovered) = (Vec::new(), None);
    let order = flat(options, favourites);
    // The wheel over the closed button steps through the flat order, one notch per step.
    let button_rect = ui.available_rect_before_wrap();
    let wheel = ui.input(|i| i.raw_scroll_delta.y);
    if wheel != 0.0 && ui.rect_contains_pointer(egui::Rect::from_min_size(button_rect.min, egui::vec2(width, ui.spacing().interact_size.y))) {
        if let Some(pos) = order.iter().position(|m| m == current) {
            let next = if wheel < 0.0 { (pos + 1).min(order.len().saturating_sub(1)) } else { pos.saturating_sub(1) };
            if next != pos {
                *current = order[next];
                chosen.push(order[next]);
            }
        }
    }
    let label_of = |m: BlendMode| options.iter().find(|(o, _)| *o == m).map(|(_, l)| *l).unwrap_or_else(|| m.label());
    egui::ComboBox::from_id_salt(id).selected_text(label).width(width).height(480.0).show_ui(ui, |ui| {
        ui.set_min_width(width.max(180.0));
        // A typed letter jumps to the next mode whose (translated) label starts with it.
        let typed: Option<char> = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Text(s) => s.chars().next().filter(|c| c.is_alphabetic()),
                _ => None,
            })
        });
        if let Some(c) = typed {
            let c = c.to_lowercase().next().unwrap_or(c);
            let pos = order.iter().position(|m| m == current).unwrap_or(0);
            let next = order
                .iter()
                .cycle()
                .skip(pos + 1)
                .take(order.len())
                .find(|m| tl!(label_of(**m)).chars().next().is_some_and(|f| f.to_lowercase().next() == Some(c)));
            if let Some(m) = next {
                *current = *m;
                chosen.push(*m);
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            ui.close();
        }
        let mut row = |ui: &mut Ui, m: BlendMode, favourites: &mut Vec<BlendMode>| {
            ui.horizontal(|ui| {
                let fav = favourites.contains(&m);
                let icon = if fav { "star-filled" } else { "star" };
                let star = crate::icons::button(ui, icon, 18.0, fav, if fav { tl!("Remove from favourites") } else { tl!("Add to favourites") });
                if star.clicked() {
                    if fav {
                        favourites.retain(|f| *f != m);
                    } else {
                        favourites.push(m);
                    }
                }
                let selected = *current == m;
                let r = ui.selectable_label(selected, RichText::new(tl!(label_of(m))).color(t.text));
                if r.hovered() {
                    hovered = Some(m);
                }
                if r.clicked() {
                    *current = m;
                    chosen.push(m);
                    ui.close();
                }
            });
        };
        let favs: Vec<BlendMode> = favourites.iter().copied().filter(|m| options.iter().any(|(o, _)| o == m)).collect();
        if !favs.is_empty() {
            ui.label(RichText::new(tl!("Favourites")).small().color(t.text_dim));
            for m in favs {
                row(ui, m, favourites);
            }
            ui.separator();
        }
        if options.iter().any(|(m, _)| *m == BlendMode::PassThrough) {
            row(ui, BlendMode::PassThrough, favourites);
            ui.separator();
        }
        for (name, modes) in families() {
            let present: Vec<BlendMode> = modes.into_iter().filter(|m| options.iter().any(|(o, _)| o == m)).collect();
            if present.is_empty() {
                continue;
            }
            ui.label(RichText::new(tl!(name)).small().color(t.text_dim));
            for m in present {
                row(ui, m, favourites);
            }
            ui.add_space(2.0);
        }
    });
    (chosen, hovered)
}
