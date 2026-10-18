//! View › Padding Color and View › Show All.
//!
//! The padding is the pasteboard round the canvas. Its colour comes from the theme (Default),
//! from the transparency checkerboard's light or dark square, from a fixed grey, or from a custom
//! colour, all stored in Preferences › Interface (`interface.canvasColor`), which the menu here
//! sets through `prefs.set` like the pasteboard's own context menu does.
//!
//! Show All reveals the layers' pixels beyond the canvas round it (the Crop tool's beyond-canvas
//! rendering, kept on). Keep Padding in Show All (`interface.keepPaddingInShowAll`) leaves the
//! padding colour under those pixels instead of the transparency checkerboard.

use redrob_photo_engine::prefs::CanvasColor;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// `view.paddingColor.<CanvasColor name>` rows, in menu order.
pub const COLORS: [(&str, CanvasColor); 8] = [
    ("view.paddingColor.default", CanvasColor::Default),
    ("view.paddingColor.black", CanvasColor::Black),
    ("view.paddingColor.darkGray", CanvasColor::DarkGray),
    ("view.paddingColor.mediumGray", CanvasColor::MediumGray),
    ("view.paddingColor.lightGray", CanvasColor::LightGray),
    ("view.paddingColor.lightCheck", CanvasColor::LightCheck),
    ("view.paddingColor.darkCheck", CanvasColor::DarkCheck),
    ("view.paddingColor.custom", CanvasColor::Custom),
];

pub const SELECT_CUSTOM: &str = "view.paddingColor.selectCustomColor";
pub const KEEP_PADDING: &str = "view.paddingColor.keepPaddingInShowAll";
pub const SHOW_ALL: &str = "view.showAll";

fn color_of(id: &str) -> Option<CanvasColor> {
    COLORS.iter().find(|(i, _)| *i == id).map(|(_, c)| *c)
}

/// Menu ids handled here.
pub fn handles(id: &str) -> bool {
    color_of(id).is_some() || matches!(id, SELECT_CUSTOM | KEEP_PADDING | SHOW_ALL)
}

/// Whether Show All is on for the document at `idx` (the view option is per shell, so it is on
/// for every document when on).
pub fn show_all(app: &PhotocraftApp, idx: usize) -> bool {
    app.ui.view.show_all && app.session.documents().get(idx).is_some()
}

/// Keep Padding in Show All.
pub fn keep_padding(app: &PhotocraftApp) -> bool {
    app.session.prefs().interface.keep_padding_in_show_all
}

pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    if let Some(c) = color_of(id) {
        return Some(app.session.prefs().interface.canvas_color == c);
    }
    match id {
        KEEP_PADDING => Some(keep_padding(app)),
        SHOW_ALL => Some(app.ui.view.show_all),
        _ => None,
    }
}

fn flag(p: &Value, current: bool) -> bool {
    p.get("on").and_then(Value::as_bool).unwrap_or(!current)
}

/// Runs a padding or Show All command. `None` when `id` isn't ours.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if let Some(c) = color_of(id) {
        return Some(app.run("prefs.set", json!({"values": {"interface.canvasColor": c.name()}})).map(|_| json!({"paddingColor": c.name()})));
    }
    Some(match id {
        SELECT_CUSTOM => {
            let rgb =
                redrob_photo_engine::prefs::parse_hex(&app.session.prefs().interface.canvas_custom_color).map_or([0.5; 3], |c| c.map(|v| f32::from(v) / 255.0));
            let _ = crate::color_picker_ui::open_for_target(app, "pasteboard", "Color Picker (Custom Canvas Color)", rgb);
            Ok(json!({"dialog": "colorPicker"}))
        }
        KEEP_PADDING => {
            let on = flag(params, keep_padding(app));
            app.run("prefs.set", json!({"values": {"interface.keepPaddingInShowAll": on}})).map(|_| json!(on))
        }
        SHOW_ALL => {
            let on = flag(params, app.ui.view.show_all);
            app.ui.view.show_all = on;
            Ok(json!(on))
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_color_rows_set_the_preference_and_show_all_toggles() {
        let mut app = PhotocraftApp::new(redrob_photo_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        assert_eq!(checked(&app, "view.paddingColor.default"), Some(true));
        menu(&mut app, "view.paddingColor.darkCheck", &json!({})).unwrap().unwrap();
        assert_eq!(app.session.prefs().interface.canvas_color, CanvasColor::DarkCheck);
        assert_eq!(checked(&app, "view.paddingColor.darkCheck"), Some(true));
        assert_eq!(checked(&app, "view.paddingColor.default"), Some(false));
        let [_, d] = app.session.prefs().transparency_and_gamut.colors();
        assert_eq!(crate::prefs_ui::pasteboard_color(&app), Some(egui::Color32::from_rgb(d[0], d[1], d[2])));
        assert_eq!(checked(&app, SHOW_ALL), Some(false));
        menu(&mut app, SHOW_ALL, &json!({})).unwrap().unwrap();
        assert!(app.ui.view.show_all && show_all(&app, 0));
        menu(&mut app, KEEP_PADDING, &json!({"on": true})).unwrap().unwrap();
        assert!(keep_padding(&app));
        assert!(!handles("view.zoomIn"));
    }
}
