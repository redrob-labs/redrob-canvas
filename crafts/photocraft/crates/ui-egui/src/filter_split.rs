//! Split View for the filter dialogs (PH-FLT-002): the canvas shows the unfiltered image on one
//! side of a divider and the live preview on the other, so a setting can be judged against the
//! original without toggling Preview. The divider is a line across the image, dragged on the
//! canvas; Shift+click turns it from vertical to horizontal (and back) and puts it under the
//! pointer. The dialog's checkbox and orientation menu drive the same state, which lives in the
//! dialog's private `__split*` fields and is remembered in `ToolOptions` between dialogs.
//!
//! The splice happens on the filtered proxy the preview worker already computed
//! (`canvas::apply_filter_split`): the pixels the filter changed are put back from the unfiltered
//! proxy on the original's side, layer by layer, so moving the divider never reruns the filter.

use egui::{Color32, Pos2, Stroke};
use redrob_photo_doc::{Document, LayerContent};
use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;

/// Private dialog fields (stripped from the command's params by `filter_dialog::params_of`).
pub(crate) const ON: &str = "__split";
pub(crate) const POS: &str = "__splitPos";
pub(crate) const HORIZONTAL: &str = "__splitHorizontal";
const DRAG: &str = "__splitDrag";

/// How close (points) the pointer must be to the divider to grab it.
const GRAB: f32 = 8.0;

/// Where the divider sits: `pos` is its position as a fraction of the image along its axis
/// (0..1), `horizontal` whether it runs across (the original above it) rather than down (the
/// original to its left).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Split {
    pub pos: f32,
    pub horizontal: bool,
}

impl Split {
    /// The divider's position in document pixels for an image `size` wide / high.
    fn at(self, size: [u32; 2]) -> f32 {
        let extent = if self.horizontal { size[1] } else { size[0] } as f32;
        (self.pos.clamp(0.0, 1.0) * extent).round()
    }
}

/// Whether the dialog with fields `f` offers Split View: a Filter menu dialog with a live
/// preview. Image › Rotation › Arbitrary and the like keep their plain preview, and Select and Mask
/// has its own view modes.
pub(crate) fn supports(f: &Map<String, Value>) -> bool {
    f.contains_key("__preview")
        && f.contains_key("__filter")
        && !crate::select_mask_view::owns(f)
        && f.get("__command").and_then(Value::as_str).is_some_and(|c| c.starts_with("filter."))
}

/// Whether the dialog with fields `f` has Split View on (so the canvas owns the divider).
pub fn owns(f: &Map<String, Value>) -> bool {
    supports(f) && f.get(ON).and_then(Value::as_bool) == Some(true) && f.get("__preview").and_then(Value::as_bool) == Some(true)
}

/// The divider the dialog with fields `f` asks for, or `None` with Split View off.
pub(crate) fn of(f: &Map<String, Value>) -> Option<Split> {
    if !owns(f) {
        return None;
    }
    let pos = f.get(POS).and_then(Value::as_f64).map(|v| v as f32).filter(|v| v.is_finite()).unwrap_or(0.5).clamp(0.0, 1.0);
    Some(Split { pos, horizontal: f.get(HORIZONTAL).and_then(Value::as_bool).unwrap_or(false) })
}

/// The divider the top filter dialog asks for, or `None`.
pub(crate) fn current(app: &PhotocraftApp) -> Option<Split> {
    app.ui.dialogs.iter().rev().find(|d| d.fields.contains_key("__filter")).and_then(|d| of(&d.fields))
}

/// Seed a new filter dialog's Split View fields from the last dialog's (`ToolOptions`).
pub(crate) fn init(app: &PhotocraftApp, f: &mut Map<String, Value>) {
    if !supports(f) {
        return;
    }
    let o = &app.ui.tool_options;
    f.insert(ON.into(), json!(o.filter_split));
    f.insert(POS.into(), json!(o.filter_split_pos.clamp(0.0, 1.0)));
    f.insert(HORIZONTAL.into(), json!(o.filter_split_horizontal));
}

/// Remember a dialog's Split View for the next one.
pub(crate) fn remember(app: &mut PhotocraftApp, f: &Map<String, Value>) {
    if !supports(f) {
        return;
    }
    let o = &mut app.ui.tool_options;
    o.filter_split = f.get(ON).and_then(Value::as_bool).unwrap_or(false);
    if let Some(pos) = f.get(POS).and_then(Value::as_f64).map(|v| v as f32).filter(|v| v.is_finite()) {
        o.filter_split_pos = pos.clamp(0.0, 1.0);
    }
    o.filter_split_horizontal = f.get(HORIZONTAL).and_then(Value::as_bool).unwrap_or(false);
}

/// The dialog's Split View row: the checkbox and, when on, the divider's orientation.
pub(crate) fn controls(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    if !supports(f) {
        return;
    }
    let mut on = f.get(ON).and_then(Value::as_bool).unwrap_or(false);
    let mut horizontal = f.get(HORIZONTAL).and_then(Value::as_bool).unwrap_or(false);
    ui.horizontal(|ui| {
        crate::widgets::checkbox(ui, &mut on, tl!("Split View"));
        if on {
            let opts = [(false, tl!("Vertical")), (true, tl!("Horizontal"))];
            crate::widgets::dropdown(ui, "flt-split-direction", &mut horizontal, &opts, 110.0);
        }
    });
    f.insert(ON.into(), json!(on));
    f.insert(HORIZONTAL.into(), json!(horizontal));
    if !f.contains_key(POS) {
        f.insert(POS.into(), json!(0.5));
    }
}

/// The divider's two ends on screen, for an image `size` wide / high.
fn ends(split: Split, size: [u32; 2], xf: &ViewXform) -> (Pos2, Pos2) {
    let at = split.at(size);
    if split.horizontal { (xf.to_screen(0.0, at), xf.to_screen(size[0] as f32, at)) } else { (xf.to_screen(at, 0.0), xf.to_screen(at, size[1] as f32)) }
}

/// Distance (points) from `p` to the segment `a`–`b`.
fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (p - (a + ab * t)).length()
}

/// The pointer on the free image under a filter dialog with Split View on: `over` is where it is,
/// `press` where the primary button is held (`dialogs::free_press`), both in screen points.
/// Returns whether the divider took it (the pointer is on the divider, dragging it, or Shift is
/// held to place it); otherwise the canvas goes on as without Split View.
pub fn canvas(app: &mut PhotocraftApp, ctx: &egui::Context, xf: &ViewXform, over: Pos2, press: Option<Pos2>) -> bool {
    let Some(size) = app.session.active().map(|st| [st.doc.size.width, st.doc.size.height]) else { return false };
    let Some(d) = app.ui.dialogs.last_mut().filter(|d| owns(&d.fields)) else { return false };
    let f = &mut d.fields;
    let Some(split) = of(f) else { return false };
    let (start, down, shift, space) = ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.modifiers.shift, i.key_down(egui::Key::Space)));
    if space {
        return false;
    }
    let (a, b) = ends(split, size, xf);
    let dragging = f.get(DRAG).and_then(Value::as_bool) == Some(true);
    let near = distance_to_segment(over, a, b) <= GRAB;
    if !down && press.is_none() {
        f.remove(DRAG);
        if !near && !shift {
            return false;
        }
        ctx.set_cursor_icon(if split.horizontal { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal });
        return true;
    }
    let Some(at) = press else { return dragging || near || shift };
    if start {
        // Shift+click turns the divider and puts it under the pointer; a click on it grabs it.
        if shift {
            f.insert(HORIZONTAL.into(), json!(!split.horizontal));
        } else if !near {
            return false;
        }
        f.insert(DRAG.into(), json!(true));
    } else if !dragging {
        return false;
    }
    let horizontal = f.get(HORIZONTAL).and_then(Value::as_bool).unwrap_or(split.horizontal);
    let doc = xf.to_doc(at);
    let (coord, extent) = if horizontal { (doc[1], f64::from(size[1])) } else { (doc[0], f64::from(size[0])) };
    if extent > 0.0 {
        let pos = (coord / extent).clamp(0.0, 1.0) as f32;
        f.insert(POS.into(), json!(pos));
        app.ui.tool_options.filter_split_pos = pos;
        app.ui.tool_options.filter_split_horizontal = horizontal;
    }
    ctx.set_cursor_icon(if horizontal { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal });
    ctx.request_repaint();
    true
}

/// Draw the divider over the image while a filter dialog has Split View on: a dark line under a
/// light one, so it reads on any image.
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(split) = current(app) else { return };
    let Some(st) = app.session.active() else { return };
    let (a, b) = ends(split, [st.doc.size.width, st.doc.size.height], xf);
    painter.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(150)));
    painter.line_segment([a, b], Stroke::new(1.0, Color32::from_white_alpha(235)));
}

/// The half of the image (document pixels, unbounded along the divider) that shows the original.
fn original_side(split: Split, size: [u32; 2]) -> Rect {
    const FAR: i32 = i32::MAX / 4;
    let at = split.at(size) as i32;
    if split.horizontal { Rect { x0: -FAR, y0: -FAR, x1: FAR, y1: at } } else { Rect { x0: -FAR, y0: -FAR, x1: at, y1: FAR } }
}

/// Whether two surfaces hold different pixels (by tile identity, as the filter leaves untouched
/// tiles shared).
fn differs(a: &Surface, b: &Surface) -> bool {
    a.format() != b.format() || a.tiles().count() != b.tiles().count() || a.tiles().any(|(c, t)| b.tile(*c).is_none_or(|o| !std::sync::Arc::ptr_eq(o, t)))
}

/// Put the unfiltered pixels of `before` back into `after` over `side`, where they differ.
fn restore(before: &Surface, after: &mut Surface, side: Rect) {
    if before.format() != after.format() || !differs(before, after) {
        return;
    }
    let area = before.content_bounds().union(&after.content_bounds()).intersect(&side);
    if area.is_empty() {
        return;
    }
    let data = before.read_region(area);
    after.write_region(area, &data);
    after.prune();
}

/// `filtered` with the unfiltered pixels of `original` (the same proxy before the filter) put
/// back on the original's side of the divider: every pixel layer, smart object cache, layer mask
/// and alpha channel the filter changed. Documents of different sizes are left alone.
pub(crate) fn splice(original: &Document, filtered: &Document, split: Split) -> Document {
    let mut out = filtered.clone();
    if original.size != filtered.size {
        return out;
    }
    let side = original_side(split, [out.size.width, out.size.height]);
    let ids: Vec<redrob_photo_doc::LayerId> = original.walk().into_iter().map(|(_, _, l)| l.id).collect();
    for id in ids {
        let Some(before) = original.layer(id) else { continue };
        let Some(after) = out.layer_mut(id) else { continue };
        match (&before.content, &mut after.content) {
            (LayerContent::Raster(b), LayerContent::Raster(a)) => restore(b, a, side),
            (LayerContent::Smart(b), LayerContent::Smart(a)) => {
                if let (Some(b), Some(a)) = (b.cache.as_ref(), a.cache.as_mut()) {
                    restore(b, a, side);
                }
            }
            _ => {}
        }
        if let (Some(b), Some(a)) = (before.mask.as_ref(), after.mask.as_mut()) {
            restore(&b.surface, &mut a.surface, side);
        }
    }
    for (b, a) in original.channels.iter().zip(out.channels.iter_mut()) {
        restore(&b.surface, &mut a.surface, side);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_divider_sits_at_its_fraction_of_the_image() {
        let v = Split { pos: 0.25, horizontal: false };
        assert_eq!(v.at([200, 100]), 50.0);
        let h = Split { pos: 0.25, horizontal: true };
        assert_eq!(h.at([200, 100]), 25.0);
        // Out-of-range and corrupt positions clamp to the image.
        assert_eq!(Split { pos: 7.0, horizontal: false }.at([200, 100]), 200.0);
        assert_eq!(Split { pos: -1.0, horizontal: true }.at([200, 100]), 0.0);
        let side = original_side(v, [200, 100]);
        assert!(side.contains(10, 50) && !side.contains(60, 50), "{side:?}");
        let side = original_side(h, [200, 100]);
        assert!(side.contains(100, 10) && !side.contains(100, 40), "{side:?}");
    }

    #[test]
    fn only_filter_dialogs_with_split_on_own_the_divider() {
        let fields = |cmd: &str, on: bool| json!({"__preview": true, "__filter": true, "__command": cmd, ON: on}).as_object().cloned().unwrap();
        assert!(supports(&fields("filter.blur.gaussianBlur", false)));
        assert!(!owns(&fields("filter.blur.gaussianBlur", false)));
        assert!(owns(&fields("filter.blur.gaussianBlur", true)));
        assert_eq!(of(&fields("filter.blur.gaussianBlur", true)), Some(Split { pos: 0.5, horizontal: false }));
        // Rotate Canvas previews but is not a filter; without a preview there is nothing to split.
        assert!(!supports(&fields("image.rotation.arbitrary", true)));
        assert!(!supports(json!({"__filter": true, "__command": "filter.blur.gaussianBlur"}).as_object().unwrap()));
    }

    #[test]
    fn the_splice_keeps_the_original_on_one_side_and_the_filter_on_the_other() {
        let mut app = PhotocraftApp::new(redrob_photo_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 32})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
        let st = app.session.active().unwrap();
        let original = st.doc.clone();
        let filtered =
            crate::filter_dialog::preview_document(&original, st.active_layer, "filter.adjust.desaturate", &json!({"mode": "luminance"}), 1).unwrap();
        let layer = st.active_layer.unwrap();
        let read = |doc: &Document, x: i32, y: i32| doc.layer(layer).unwrap().surface().unwrap().read_region(Rect { x0: x, y0: y, x1: x + 1, y1: y + 1 });
        let red = read(&original, 10, 10);
        let grey = read(&filtered, 10, 10);
        assert_ne!(red, grey, "the filter changed the pixels");

        let spliced = splice(&original, &filtered, Split { pos: 0.5, horizontal: false });
        assert_eq!(read(&spliced, 10, 10), red, "left of the divider: the original");
        assert_eq!(read(&spliced, 50, 10), grey, "right of the divider: the filter");
        let spliced = splice(&original, &filtered, Split { pos: 0.5, horizontal: true });
        assert_eq!(read(&spliced, 50, 5), red, "above the divider: the original");
        assert_eq!(read(&spliced, 50, 25), grey, "below the divider: the filter");
        // An unchanged layer (the Background) is left shared, not rewritten.
        let bg = original.layers[0].id;
        let same = |doc: &Document| doc.layer(bg).unwrap().surface().unwrap().tiles().map(|(c, t)| (*c, std::sync::Arc::as_ptr(t))).collect::<Vec<_>>();
        assert_eq!(same(&original), same(&spliced));
    }

    #[test]
    fn the_pointer_grabs_only_the_divider() {
        let xf = ViewXform {
            rect: egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(200.0, 100.0)),
            zoom: 1.0,
            center: [100.0, 50.0],
            flip: false,
            rotation: 0.0,
            aspect: 1.0,
        };
        let (a, b) = ends(Split { pos: 0.5, horizontal: false }, [200, 100], &xf);
        assert_eq!((a.x, b.x), (100.0, 100.0));
        assert!(distance_to_segment(Pos2::new(103.0, 20.0), a, b) <= GRAB);
        assert!(distance_to_segment(Pos2::new(130.0, 20.0), a, b) > GRAB);
        // Beyond the divider's ends the distance grows too.
        assert!(distance_to_segment(Pos2::new(100.0, 140.0), a, b) > GRAB);
    }
}
