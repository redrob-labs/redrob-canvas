//! The Dynamic Brush tool: the pointer drags a weight on a string, so the stroke lags behind it,
//! rounding corners and smoothing jitter. Mass sets how slowly the weight follows, drag how fast
//! its motion dies away; the stroke ends where the weight came to rest, not where the pointer let
//! go. A pure pointer-event filter ([`filter_event`]) in front of the Brush's stroke handling.

use crate::{PhotocraftApp, canvas::ToolEvent, state::Tool};

/// The weight's state during a stroke.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Dyna {
    pub pos: [f64; 2],
    pub vel: [f64; 2],
}

impl Dyna {
    /// Start at the press.
    pub fn at(p: [f64; 2]) -> Self {
        Self { pos: p, vel: [0.0; 2] }
    }

    /// One pointer step towards `target`: the string pulls with a force proportional to the
    /// distance, the weight's `mass` (0..1, heavier = slower) resists it, `drag` (0..1) damps the
    /// velocity. Returns the weight's new position.
    pub fn step(&mut self, target: [f64; 2], mass: f64, drag: f64) -> [f64; 2] {
        let mass = mass.clamp(0.0, 1.0);
        let drag = drag.clamp(0.0, 1.0);
        // Light weights snap to the pointer; a full mass takes a tenth of the distance per step.
        let k = 1.0 - 0.9 * mass;
        let acc = [(target[0] - self.pos[0]) * k, (target[1] - self.pos[1]) * k];
        self.vel = [(self.vel[0] + acc[0]) * (1.0 - drag), (self.vel[1] + acc[1]) * (1.0 - drag)];
        // With no damping the weight would oscillate; limit a step to the remaining distance.
        let remaining = ((target[0] - self.pos[0]).powi(2) + (target[1] - self.pos[1]).powi(2)).sqrt();
        let speed = (self.vel[0].powi(2) + self.vel[1].powi(2)).sqrt();
        if speed > remaining && speed > 0.0 {
            let f = remaining / speed;
            self.vel = [self.vel[0] * f, self.vel[1] * f];
        }
        self.pos = [self.pos[0] + self.vel[0], self.pos[1] + self.vel[1]];
        self.pos
    }

    /// Let the weight settle onto `target` at the release: the stroke ends there.
    pub fn settle(&mut self, target: [f64; 2]) -> [f64; 2] {
        self.pos = target;
        self.vel = [0.0; 2];
        self.pos
    }
}

/// Pointer events of the Dynamic Brush pass through the weight.
pub fn filter_event(app: &mut PhotocraftApp, ev: ToolEvent) -> ToolEvent {
    if app.ui.tool != Tool::DynamicBrush {
        app.dyna = None;
        return ev;
    }
    let (mass, drag) = (f64::from(app.ui.tool_options.dyna_mass), f64::from(app.ui.tool_options.dyna_drag));
    match ev {
        ToolEvent::Down { x, y, .. } => {
            app.dyna = Some(Dyna::at([x, y]));
            ev
        }
        ToolEvent::Move { x, y, pressure } => match app.dyna.as_mut() {
            Some(d) => {
                let p = d.step([x, y], mass, drag);
                ToolEvent::Move { x: p[0], y: p[1], pressure }
            }
            None => ev,
        },
        ToolEvent::Up { x, y } => {
            let p = app.dyna.take().map_or([x, y], |mut d| d.settle([x, y]));
            ToolEvent::Up { x: p[0], y: p[1] }
        }
    }
}

/// The Dynamic Brush's options: mass and drag.
pub fn options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if app.ui.tool != Tool::DynamicBrush {
        return;
    }
    let o = &mut app.ui.tool_options;
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!("Mass")).color(t.text_dim).size(12.0));
    crate::widgets::value_field(ui, &mut o.dyna_mass, 0.0..=1.0, "", 52.0).on_hover_text(tl!("How heavily the stroke lags behind the pointer"));
    ui.label(egui::RichText::new(tl!("Drag")).color(t.text_dim).size(12.0));
    crate::widgets::value_field(ui, &mut o.dyna_drag, 0.0..=1.0, "", 52.0).on_hover_text(tl!("How quickly the stroke's motion dies away"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_weight_lags_and_settles() {
        let mut d = Dyna::at([0.0, 0.0]);
        let p = d.step([100.0, 0.0], 0.8, 0.5);
        assert!(p[0] > 0.0 && p[0] < 100.0, "{p:?}");
        let mut light = Dyna::at([0.0, 0.0]);
        let q = light.step([100.0, 0.0], 0.0, 0.0);
        assert!((q[0] - 100.0).abs() < 1e-9, "no mass, no drag: the weight is at the pointer: {q:?}");
        // Repeated steps towards a fixed target converge without overshooting past it.
        let mut d = Dyna::at([0.0, 0.0]);
        let mut last = 0.0;
        for _ in 0..200 {
            let p = d.step([50.0, 0.0], 0.9, 0.2);
            assert!(p[0] <= 50.0 + 1e-6 && p[0] >= last - 1e-6, "{p:?}");
            last = p[0];
        }
        assert!((last - 50.0).abs() < 0.5, "{last}");
        assert_eq!(d.settle([60.0, 0.0]), [60.0, 0.0]);
    }
}
