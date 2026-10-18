//! Magic Wand press-and-drag: the threshold (tolerance) follows the pointer while the button is
//! held, so a region can be grown or shrunk by dragging right or left before releasing. One
//! screen pixel of horizontal drag is one tolerance level (0..255); the runs of the wand during
//! one press coalesce into one History step, and the final tolerance is kept in the tool options.

use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

/// A press of the Magic Wand that is still held.
#[derive(Clone, Debug, PartialEq)]
pub struct WandDrag {
    /// Document point of the press (the seed).
    pub seed: [f64; 2],
    /// Tolerance when the press began.
    pub start_tolerance: f32,
    /// Tolerance of the last run.
    pub tolerance: f32,
    /// Selection combine mode of the press.
    pub mode: &'static str,
    /// Coalescing key of this press's History step.
    key: String,
}

static PRESSES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Tolerance for a horizontal drag of `dx_doc` document pixels at `zoom` from `start`: one screen
/// pixel per level, clamped to the tool's 0..255.
pub fn tolerance_for(start: f32, dx_doc: f64, zoom: f32) -> f32 {
    let px = dx_doc * f64::from(zoom.max(1e-3));
    (f64::from(start) + px).round().clamp(0.0, 255.0) as f32
}

fn run(app: &mut PhotocraftApp, d: &WandDrag) {
    let o = app.ui.tool_options.clone();
    let _ = app.run(
        "select.magicWand",
        json!({"x": d.seed[0].floor(), "y": d.seed[1].floor(), "tolerance": d.tolerance, "contiguous": o.contiguous, "antiAlias": o.anti_alias, "sampleAllLayers": o.sample_all_layers, "diagonal": o.wand_diagonal, "criterion": o.wand_criterion, "mode": d.mode, "coalesce": d.key}),
    );
}

/// Handles one canvas pointer event for the Magic Wand; true when it was consumed.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    match ev {
        ToolEvent::Down { x, y, .. } => {
            // ⌘ with any tool is the Move tool (and a ⌘-drag over a guide moves the guide, #2690).
            if app.ui.tool != Tool::MagicWand || mods.command || app.drag.is_some() || app.session.active().is_none() {
                return false;
            }
            let n = PRESSES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let t = app.ui.tool_options.tolerance;
            let d = WandDrag { seed: [x, y], start_tolerance: t, tolerance: t, mode: crate::canvas::selection_mode(app, mods), key: format!("magic-wand-{n}") };
            run(app, &d);
            app.wand_drag = Some(d);
            true
        }
        ToolEvent::Move { x, .. } => {
            let Some(mut d) = app.wand_drag.clone() else { return false };
            let t = tolerance_for(d.start_tolerance, x - d.seed[0], app.point_zoom());
            if t != d.tolerance {
                d.tolerance = t;
                run(app, &d);
                app.ui.tool_options.tolerance = t;
                app.ui.status = format!("{}: {t:.0}", tl!("Tolerance"));
                app.ui.status_error = false;
                app.wand_drag = Some(d);
            }
            true
        }
        ToolEvent::Up { .. } => {
            let Some(d) = app.wand_drag.take() else { return false };
            app.ui.tool_options.tolerance = d.tolerance;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use redrob_photo_geom::Rect;
    use serde_json::json;

    use super::*;
    use crate::canvas::tool_event;

    /// A 64×64 white document with a soft grey ramp on the right half.
    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(redrob_photo_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.session
            .edit("ramp", |doc, a| {
                let surf = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
                for x in 32..64 {
                    let g = 1.0 - (x - 32) as f32 / 64.0;
                    surf.fill_rect(Rect::new(x, 0, x + 1, 64), &[g, g, g, 1.0]);
                }
                Ok(())
            })
            .unwrap();
        app.sync_views();
        app.ui.tool = Tool::MagicWand;
        app
    }

    fn selected_width(app: &PhotocraftApp) -> u32 {
        app.session.active().unwrap().doc.selection.as_ref().map_or(0, |s| s.content_bounds().width())
    }

    #[test]
    fn dragging_right_grows_the_tolerance_in_one_history_step() {
        let mut app = app();
        app.ui.tool_options.tolerance = 0.0;
        let before = app.session.active().unwrap().history.past_len();
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
        let narrow = selected_width(&app);
        assert!(narrow >= 32 && narrow < 64, "{narrow}");
        let zoom = app.point_zoom();
        tool_event(&mut app, ToolEvent::Move { x: 10.0 + 100.0 / f64::from(zoom), y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
        assert_eq!(app.ui.tool_options.tolerance, 100.0);
        assert!(selected_width(&app) > narrow);
        tool_event(&mut app, ToolEvent::Up { x: 10.0, y: 10.0 }, egui::Modifiers::NONE);
        assert!(app.wand_drag.is_none());
        assert_eq!(app.session.active().unwrap().history.past_len(), before + 1, "the runs of one press are one step");
        assert_eq!(tolerance_for(32.0, -100.0, 1.0), 0.0);
        assert_eq!(tolerance_for(32.0, 10.0, 2.0), 52.0);
    }
}
