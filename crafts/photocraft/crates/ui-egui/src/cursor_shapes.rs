//! Custom painting cursor shapes (Preferences › Cursors › Cursor shape / Outline shape): the
//! pointer as an arrow, a crosshair, a small dot, nothing, a triangle or the pixel under it, with
//! the brush outline as a circle, the tip's preview, or a circle flattened by the stylus tilt,
//! optionally hidden while painting and sized by the pressure.

use egui::{Color32, CursorIcon, Painter, Pos2, Stroke, pos2, vec2};
use redrob_photo_engine::prefs::{CursorShape, Cursors, OutlineShape};
use redrob_photo_paint::brush::BrushSettings;

use crate::{PhotocraftApp, canvas::ViewXform};

fn two_tone<R>(painter: &Painter, f: impl Fn(Stroke) -> R) {
    let _ = painter;
    f(Stroke::new(3.0, Color32::from_black_alpha(140)));
    f(Stroke::new(1.0, Color32::WHITE));
}

/// The outline for `shape` around `at`: `radius` is the tip's screen radius, `tilt` the stylus
/// tilt in degrees (x, y). Draws nothing for `NoOutline`; the tip preview falls back to the
/// circle when the tip has no traced outline.
fn outline(painter: &Painter, xf: &ViewXform, at: Pos2, brush: &BrushSettings, radius: f32, shape: OutlineShape, tilt: [f32; 2]) {
    match shape {
        OutlineShape::NoOutline => {}
        OutlineShape::Circle => two_tone(painter, |s| painter.circle_stroke(at, radius, s)),
        OutlineShape::Preview => {
            let size = radius * 2.0 / xf.zoom.max(1e-6);
            match crate::brush_cursor::rings(painter.ctx(), brush, true) {
                Some(rings) => {
                    let axes = crate::brush_cursor::tip_axes(xf, size);
                    for ring in rings.iter() {
                        let pts: Vec<Pos2> = ring.iter().map(|p| at + axes[0] * p[0] + axes[1] * p[1]).collect();
                        if pts.len() >= 2 {
                            two_tone(painter, |s| painter.add(egui::Shape::closed_line(pts.clone(), s)));
                        }
                    }
                }
                None => two_tone(painter, |s| painter.circle_stroke(at, radius, s)),
            }
        }
        OutlineShape::Tilt => {
            // The circle seen in perspective: flattened across the tilt direction by its angle.
            let mag = tilt[0].hypot(tilt[1]).clamp(0.0, 90.0);
            let squash = (mag.to_radians()).cos().max(0.15);
            let dir = if mag < 1e-3 { 0.0 } else { tilt[1].atan2(tilt[0]) };
            let (s, c) = dir.sin_cos();
            const N: usize = 48;
            let pts: Vec<Pos2> = (0..N)
                .map(|i| {
                    let t = i as f32 / N as f32 * std::f32::consts::TAU;
                    let (lx, ly) = (radius * t.cos() * squash, radius * t.sin());
                    pos2(at.x + lx * c - ly * s, at.y + lx * s + ly * c)
                })
                .collect();
            two_tone(painter, |st| painter.add(egui::Shape::closed_line(pts.clone(), st)));
            // The tilt direction as a short tick.
            if mag >= 1e-3 {
                let tip = at + vec2(c, s) * radius * 1.3;
                two_tone(painter, |st| painter.line_segment([at, tip], st));
            }
        }
    }
}

/// The custom painting cursor at `at`, or `None` when the shape is the tool icon (the standard
/// cursor preferences then decide). `radius` is the tip's screen radius at the nominal size,
/// `pressure` the current stylus pressure (1 for a mouse), `painting` whether the button is down.
pub fn custom(
    app: &PhotocraftApp,
    painter: &Painter,
    xf: &ViewXform,
    at: Pos2,
    brush: &BrushSettings,
    radius: f32,
    pressure: f32,
    painting: bool,
) -> Option<CursorIcon> {
    let cur: &Cursors = &app.session.prefs().cursors;
    if cur.cursor_shape == CursorShape::ToolIcon {
        return None;
    }
    // The outline first, so the pointer shape draws over it.
    if !painting || cur.show_outline_while_painting {
        let r = if cur.effective_outline_size && brush.pressure_size { radius * pressure.clamp(0.0, 1.0) } else { radius };
        let tilt = app.stylus.sample().map_or([0.0, 0.0], |s| [s.tilt_x, s.tilt_y]);
        outline(painter, xf, at, brush, r.max(1.0), cur.outline_shape, tilt);
    }
    Some(match cur.cursor_shape {
        CursorShape::ToolIcon => CursorIcon::Default,
        CursorShape::Arrow => CursorIcon::Default,
        CursorShape::Crosshair => crate::tool_cursor::crosshair(painter, at, 6.0, 0.0),
        CursorShape::SmallDot => {
            painter.circle_filled(at, 2.5, Color32::from_black_alpha(160));
            painter.circle_filled(at, 1.5, Color32::WHITE);
            CursorIcon::None
        }
        CursorShape::NoCursor => CursorIcon::None,
        CursorShape::TriangleRight | CursorShape::TriangleLeft => {
            let d = if cur.cursor_shape == CursorShape::TriangleRight { 1.0 } else { -1.0 };
            let pts = vec![at, pos2(at.x - 10.0 * d, at.y - 5.0), pos2(at.x - 10.0 * d, at.y + 5.0)];
            painter.add(egui::Shape::convex_polygon(pts.clone(), Color32::WHITE, Stroke::new(1.0, Color32::from_black_alpha(200))));
            CursorIcon::None
        }
        CursorShape::Pixel => {
            // The document pixel under the pointer, outlined on screen.
            let d = xf.to_doc(at);
            let a = xf.to_screen(d[0].floor() as f32, d[1].floor() as f32);
            let b = xf.to_screen(d[0].floor() as f32 + 1.0, d[1].floor() as f32 + 1.0);
            let r = egui::Rect::from_two_pos(a, b);
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_black_alpha(200)), egui::StrokeKind::Outside);
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Inside);
            CursorIcon::None
        }
    })
}
