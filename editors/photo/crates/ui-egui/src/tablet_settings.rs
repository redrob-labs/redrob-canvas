//! Tablet settings (Preferences › Tools): tilt offsets, the speed source, the input API and the
//! screen mapping, applied to every pen sample before a tool sees it; and the Tablet Tester
//! window (Window › Tablet Tester), which draws the pen's path with its pressure and tilt and
//! shows the raw numbers, so a tablet's setup can be checked without painting.

use egui::{Color32, Pos2, Stroke, pos2, vec2};
use redrob_photo_engine::prefs::{TabletApi, TabletSpeedSource};

use crate::{PhotoApp, stylus::PenSample, theme::Tokens, widgets};

/// Settings applied to every pen sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Degrees added to the reported tilt (a pen held askew reads level).
    pub tilt_offset: [f32; 2],
    pub speed_source: TabletSpeedSource,
    pub api: TabletApi,
    /// The fraction of the screen the tablet covers, `[x, y, w, h]` in 0..1; the whole screen by
    /// default. A pointer position is mapped from this area to the whole screen.
    pub screen_mapping: [f32; 4],
}

impl Default for Settings {
    fn default() -> Self {
        Self { tilt_offset: [0.0, 0.0], speed_source: TabletSpeedSource::Pointer, api: TabletApi::Auto, screen_mapping: [0.0, 0.0, 1.0, 1.0] }
    }
}

impl Settings {
    /// The settings Preferences › Tools hold.
    pub fn from_prefs(app: &PhotoApp) -> Self {
        let t = &app.session.prefs().tools;
        Self {
            tilt_offset: [t.tilt_offset_x, t.tilt_offset_y],
            speed_source: t.tablet_speed_source,
            api: t.tablet_api,
            screen_mapping: parse_mapping(&t.tablet_screen_mapping).unwrap_or([0.0, 0.0, 1.0, 1.0]),
        }
    }

    /// `sample` with the tilt offsets added (clamped back to the pen's range).
    pub fn apply(&self, sample: PenSample) -> PenSample {
        let o = self.tilt_offset;
        PenSample { tilt_x: (sample.tilt_x + o[0]).clamp(-90.0, 90.0), tilt_y: (sample.tilt_y + o[1]).clamp(-90.0, 90.0), ..sample }.sanitized()
    }

    /// A screen position (fractions of the screen, 0..1) read from the mapped area: a point at
    /// the area's left edge is the screen's left edge. The identity for the full-screen mapping.
    pub fn map_position(&self, p: [f32; 2]) -> [f32; 2] {
        let [x, y, w, h] = self.screen_mapping;
        let (w, h) = (if w.abs() < 1e-6 { 1.0 } else { w }, if h.abs() < 1e-6 { 1.0 } else { h });
        [((p[0] - x) / w).clamp(0.0, 1.0), ((p[1] - y) / h).clamp(0.0, 1.0)]
    }

    /// Is the mapping valid (a positive area inside the screen)?
    pub fn mapping_is_valid(&self) -> bool {
        let [x, y, w, h] = self.screen_mapping;
        [x, y, w, h].iter().all(|v| v.is_finite()) && w > 0.0 && h > 0.0 && x >= 0.0 && y >= 0.0 && x + w <= 1.0 + 1e-6 && y + h <= 1.0 + 1e-6
    }
}

/// `"x,y,w,h"` (fractions, or percentages when any value is above 1) as a mapping; `None` for an
/// empty or malformed string.
pub fn parse_mapping(s: &str) -> Option<[f32; 4]> {
    let v: Vec<f32> = s.split(',').map(|p| p.trim().trim_end_matches('%').parse::<f32>().ok()).collect::<Option<_>>()?;
    if v.len() != 4 {
        return None;
    }
    let scale = if v.iter().any(|x| *x > 1.0) { 0.01 } else { 1.0 };
    let m = [v[0] * scale, v[1] * scale, v[2] * scale, v[3] * scale];
    (Settings { screen_mapping: m, ..Default::default() }).mapping_is_valid().then_some(m)
}

/// The tablet tester's recording: the last samples with their positions, so the window can draw
/// pressure, tilt and the pen's path and show the raw numbers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tester {
    pub points: Vec<TestPoint>,
    /// How many points are kept.
    pub capacity: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TestPoint {
    /// Position in the tester's own area, 0..1.
    pub pos: [f32; 2],
    pub sample: PenSample,
    /// Milliseconds since the tester started.
    pub time_ms: f64,
    /// Whether the pen was a pen (not a mouse).
    pub pen: bool,
}

impl Tester {
    pub fn new(capacity: usize) -> Self {
        Self { points: Vec::new(), capacity: capacity.max(1) }
    }

    pub fn push(&mut self, p: TestPoint) {
        self.points.push(p);
        if self.points.len() > self.capacity {
            let drop = self.points.len() - self.capacity;
            self.points.drain(..drop);
        }
    }

    pub fn clear(&mut self) {
        self.points.clear();
    }

    pub fn last(&self) -> Option<&TestPoint> {
        self.points.last()
    }

    /// Samples per second over the recording (0 with fewer than two points).
    pub fn rate_hz(&self) -> f64 {
        match (self.points.first(), self.points.last()) {
            (Some(a), Some(b)) if b.time_ms > a.time_ms && self.points.len() >= 2 => (self.points.len() - 1) as f64 * 1000.0 / (b.time_ms - a.time_ms),
            _ => 0.0,
        }
    }

    /// The highest pressure seen (the pen's full press).
    pub fn max_pressure(&self) -> f32 {
        self.points.iter().map(|p| p.sample.pressure).fold(0.0, f32::max)
    }

    pub fn saw_pen(&self) -> bool {
        self.points.iter().any(|p| p.pen)
    }
}

/// Window › Tablet Tester toggles the window.
pub fn menu(app: &mut PhotoApp, id: &str) -> Option<Result<serde_json::Value, String>> {
    if id != "window.tabletTester" {
        return None;
    }
    app.ui.panels.tablet_tester = !app.ui.panels.tablet_tester;
    Some(Ok(serde_json::json!({"visible": app.ui.panels.tablet_tester})))
}

pub fn checked(app: &PhotoApp, id: &str) -> Option<bool> {
    (id == "window.tabletTester").then_some(app.ui.panels.tablet_tester)
}

/// The Tablet Tester window: a drawing area recording the pen, its path drawn with the pressure
/// as width and the tilt as a tick, the raw numbers, the sample rate, and the settings in effect.
pub fn window(app: &mut PhotoApp, ctx: &egui::Context) {
    if !app.ui.panels.tablet_tester {
        return;
    }
    let mut open = true;
    let settings = Settings::from_prefs(app);
    egui::Window::new(tl!("Tablet Tester")).id(egui::Id::new("tablet-tester")).open(&mut open).resizable(true).default_width(360.0).show(ctx, |ui| {
        let t = Tokens::get(ui.ctx());
        ui.label(egui::RichText::new(tl!("Draw in the area below to test pressure, tilt and rotation")).color(t.text_dim));
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width().max(200.0), 180.0), egui::Sense::drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, t.canvas);
        if let Some(p) = response.interact_pointer_pos().filter(|_| response.dragged() || response.drag_started()) {
            let raw = app.stylus.sample();
            let sample = settings.apply(raw.unwrap_or_default());
            let pos = [((p.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0), ((p.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0)];
            app.tablet_tester.push(TestPoint { pos, sample, time_ms: crate::gpu_canvas::now_ms(), pen: raw.is_some() });
        }
        if response.drag_stopped() {
            // A gap between strokes: keep the recording, mark the break by a zero-pressure point.
            if let Some(last) = app.tablet_tester.last().copied() {
                app.tablet_tester.push(TestPoint { sample: PenSample { pressure: 0.0, ..last.sample }, ..last });
            }
        }
        let to_screen = |q: [f32; 2]| pos2(rect.left() + q[0] * rect.width(), rect.top() + q[1] * rect.height());
        let pts = &app.tablet_tester.points;
        for w in pts.windows(2) {
            if w[0].sample.pressure <= 0.0 {
                continue;
            }
            let width = 1.0 + 7.0 * w[1].sample.pressure.clamp(0.0, 1.0);
            let color = if w[1].pen { t.accent } else { Color32::GRAY };
            painter.line_segment([to_screen(w[0].pos), to_screen(w[1].pos)], Stroke::new(width, color));
        }
        if let Some(last) = pts.last() {
            let at: Pos2 = to_screen(last.pos);
            let tilt = vec2(last.sample.tilt_x, last.sample.tilt_y) / 90.0 * 30.0;
            painter.line_segment([at, at + tilt], Stroke::new(1.5, Color32::WHITE));
            let rot = last.sample.rotation.to_radians();
            painter.line_segment([at, at + vec2(rot.sin(), -rot.cos()) * 14.0], Stroke::new(1.0, Color32::YELLOW));
        }
        ui.add_space(4.0);
        match app.tablet_tester.last() {
            Some(p) => {
                ui.label(format!(
                    "{}: {:.3}   {}: {:+.1}° / {:+.1}°   {}: {:.1}°   {}: {}",
                    tl!("Pressure"),
                    p.sample.pressure,
                    tl!("Tilt"),
                    p.sample.tilt_x,
                    p.sample.tilt_y,
                    tl!("Rotation"),
                    p.sample.rotation,
                    tl!("Device"),
                    if p.pen { tl!("Pen") } else { tl!("Mouse") }
                ));
                ui.label(format!(
                    "{}: {:.0} Hz   {}: {:.3}",
                    tl!("Sample rate"),
                    app.tablet_tester.rate_hz(),
                    tl!("Peak pressure"),
                    app.tablet_tester.max_pressure()
                ));
            }
            None => {
                ui.label(egui::RichText::new(tl!("No samples yet")).color(t.text_dim));
            }
        }
        ui.label(format!("{}: {}", tl!("Canvas input profile"), app.session.prefs().canvas_input.profile.name()));
        ui.label(format!(
            "{}: {:+.1}° / {:+.1}°   {}: {}   {}: {}   {}: {:.0}%, {:.0}%, {:.0}%, {:.0}%",
            tl!("Tilt offset"),
            settings.tilt_offset[0],
            settings.tilt_offset[1],
            tl!("Speed source"),
            settings.speed_source.name(),
            tl!("Input API"),
            settings.api.name(),
            tl!("Screen mapping"),
            settings.screen_mapping[0] * 100.0,
            settings.screen_mapping[1] * 100.0,
            settings.screen_mapping[2] * 100.0,
            settings.screen_mapping[3] * 100.0
        ));
        ui.horizontal(|ui| {
            if widgets::secondary_button(ui, tl!("Clear"), 0.0).clicked() {
                app.tablet_tester.clear();
            }
            if widgets::secondary_button(ui, tl!("Tablet Settings…"), 0.0).clicked() {
                let _ = crate::menus::invoke(app, ui.ctx(), "edit.preferences.tools", serde_json::json!({}));
            }
        });
    });
    if !open {
        app.ui.panels.tablet_tester = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_offset_tilt_and_map_the_screen() {
        let s = Settings { tilt_offset: [10.0, -5.0], ..Default::default() };
        let out = s.apply(PenSample { tilt_x: 85.0, tilt_y: 0.0, ..Default::default() });
        assert_eq!((out.tilt_x, out.tilt_y), (90.0, -5.0));
        assert_eq!(Settings::default().map_position([0.25, 0.5]), [0.25, 0.5]);
        let half = Settings { screen_mapping: [0.5, 0.0, 0.5, 1.0], ..Default::default() };
        assert_eq!(half.map_position([0.75, 0.5]), [0.5, 0.5]);
        assert_eq!(half.map_position([0.0, 0.5]), [0.0, 0.5], "outside the area clamps");
        assert!(half.mapping_is_valid());
        assert_eq!(parse_mapping("50,0,50,100"), Some([0.5, 0.0, 0.5, 1.0]));
        assert_eq!(parse_mapping("0.5, 0, 0.5, 1"), Some([0.5, 0.0, 0.5, 1.0]));
        assert_eq!(parse_mapping(""), None);
        assert_eq!(parse_mapping("0,0,2,1"), Some([0.0, 0.0, 0.02, 0.01]), "a value above 1 reads the four as percentages");
        assert_eq!(parse_mapping("0,0,150,100"), None, "past the screen");
        assert_eq!(parse_mapping("50,0,60,100"), None, "past the screen's right edge");
    }

    #[test]
    fn tester_keeps_the_last_points_and_measures_the_rate() {
        let mut t = Tester::new(3);
        for i in 0..5 {
            t.push(TestPoint {
                pos: [0.1 * i as f32, 0.0],
                sample: PenSample { pressure: 0.2 * i as f32, ..Default::default() },
                time_ms: 10.0 * f64::from(i),
                pen: i % 2 == 0,
            });
        }
        assert_eq!(t.points.len(), 3);
        assert_eq!(t.last().map(|p| p.time_ms), Some(40.0));
        assert!((t.rate_hz() - 100.0).abs() < 1e-6, "{}", t.rate_hz());
        assert!((t.max_pressure() - 0.8).abs() < 1e-6);
        assert!(t.saw_pen());
        t.clear();
        assert_eq!(t.rate_hz(), 0.0);
    }
}
