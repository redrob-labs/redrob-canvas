// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Layers panel's grouped blend-mode menu: favourites first (a star on each row toggles
//! them), then the modes by family (arithmetic, binary, darken, HSX, lighten, misc, mix, modulo,
//! negative, quadratic) with headers. A typed letter jumps to the next mode starting with it,
//! Return closes; the wheel over the button or the open list steps through the modes in the
//! Layers panel's order (Photoshop's list, then the extra families), one notch per mode.

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

/// Draw the menu. `options` are the modes the layer may take (with labels), in the order the
/// wheel, the arrow keys and ⇧+/⇧− step through; `favourites` is the user's list, edited in place
/// by the stars. Returns the modes chosen this frame (one per click, key or wheel notch; the last
/// is `current`) and the mode under the pointer in the open list. The list itself is
/// [`crate::widgets::dropdown_grouped`]: press-drag-release, type-ahead and the wheel over the
/// open list work as in every other dropdown.
pub fn menu(
    ui: &mut Ui,
    id: &str,
    current: &mut BlendMode,
    options: &[(BlendMode, &'static str)],
    favourites: &mut Vec<BlendMode>,
    width: f32,
) -> (Vec<BlendMode>, Option<BlendMode>) {
    let t = Tokens::get(ui.ctx());
    let index = |m: BlendMode| options.iter().position(|(o, _)| *o == m);
    let mut rows = |ui: &mut Ui, draw: &mut dyn FnMut(&mut Ui, usize)| -> bool {
        let mut starred = false;
        let mut row = |ui: &mut Ui, m: BlendMode, favourites: &mut Vec<BlendMode>| {
            let Some(i) = index(m) else { return };
            ui.horizontal(|ui| {
                let fav = favourites.contains(&m);
                let icon = if fav { "star-filled" } else { "star" };
                let star = crate::icons::button(ui, icon, 18.0, fav, if fav { tl!("Remove from favourites") } else { tl!("Add to favourites") });
                if star.clicked() {
                    starred = true;
                    if fav {
                        favourites.retain(|f| *f != m);
                    } else {
                        favourites.push(m);
                    }
                }
                draw(ui, i);
            });
        };
        let favs: Vec<BlendMode> = favourites.iter().copied().filter(|m| index(*m).is_some()).collect();
        if !favs.is_empty() {
            ui.label(RichText::new(tl!("Favourites")).small().color(t.text_dim));
            for m in favs {
                row(ui, m, favourites);
            }
            ui.separator();
        }
        if index(BlendMode::PassThrough).is_some() {
            row(ui, BlendMode::PassThrough, favourites);
            ui.separator();
        }
        for (name, modes) in families() {
            let present: Vec<BlendMode> = modes.into_iter().filter(|m| index(*m).is_some()).collect();
            if present.is_empty() {
                continue;
            }
            ui.label(RichText::new(tl!(name)).small().color(t.text_dim));
            for m in present {
                row(ui, m, favourites);
            }
            ui.add_space(2.0);
        }
        starred
    };
    crate::widgets::dropdown_grouped(ui, id, current, options, width, &mut rows)
}

/// The row of mode `label` in the open grouped list, for tests. A favourite is listed twice (its
/// favourites row first) and a family header may read the same as a mode, so this takes the
/// first row *button* that is in the list's unscrolled view (the 420 pt below its first row, the
/// favourites' Normal), or the first one at all when none is.
#[cfg(test)]
pub(crate) fn test_row(h: &egui_kittest::Harness<'_, crate::PhotoApp>, label: &str) -> Option<egui::Rect> {
    use egui_kittest::kittest::{NodeT, Queryable};
    let button = |n: &egui_kittest::Node<'_>| n.accesskit_node().role() == egui::accesskit::Role::Button;
    let list_top = h.query_all_by_label("Normal").filter(button).map(|n| n.rect().top()).fold(f32::INFINITY, f32::min);
    let rows: Vec<egui::Rect> = h.query_all_by_label(label).filter(button).map(|n| n.rect()).collect();
    rows.iter().copied().find(|r| r.top() >= list_top - 0.5 && r.bottom() <= list_top + 420.0 + 0.5).or_else(|| rows.first().copied())
}
