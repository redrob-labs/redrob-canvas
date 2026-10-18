//! From input points to dabs: smoothing, brush pose, path walking (spacing, airbrush build-up) and
//! the per-dab dynamics (shape, scattering, transfer, colour, texture depth).
//!
//! Everything is incremental (feed points in any chunking, get the same dabs) and deterministic:
//! jitter comes from [`crate::rng`] keyed by the stroke seed and the step/dab index.

use redrob_photo_geom::Point;

use crate::brush::{BrushSettings, Control, Dynamic, HoseSelection, ImageHose, SensorCombine, Smoothing};
use crate::rng::{rand_signed, rand01, stream};
use crate::{Dab, StrokePoint};

/// Maximum pulled-string length at 100 % smoothing, in screen pixels.
pub const MAX_STRING_PX: f64 = 100.0;
/// Stroke time over which exponential smoothing moves its fraction of the way to the pointer.
pub const SMOOTHING_STEP_MS: f64 = 16.0;

/// With Spacing unchecked, one dab per this many milliseconds of stroke time, so faster strokes
/// space their dabs further apart (Photoshop: "the speed of the cursor determines the spacing").
pub const SPEED_SPACING_MS: f64 = 8.0;

/// Longest pixel-grid segment (in pixels) before falling back to distance spacing.
const MAX_GRID_STEPS: f64 = 1e7;

/// Pointer speed, in pixels per millisecond of stroke time, at which [`Control::Velocity`] (and an
/// image hose's velocity selection) reads 1: 2000 px/s is a fast flick on any tablet.
pub const VELOCITY_FULL_PX_PER_MS: f32 = 2.0;

/// Most input points the stabilizer averages.
pub const MAX_STABILIZER_SAMPLES: usize = 200;
/// Most extra dabs a fanned corner inserts between two steps.
const MAX_FAN_DABS: u32 = 64;

/// Densest speed spacing, in pixels between dabs (a slow, long stroke never floods the buffer).
const MIN_SPEED_STEP: f64 = 0.5;
/// Maximum airbrush catch-up dabs for one input segment.
const MAX_AIRBRUSH_DABS_PER_SEGMENT: usize = 512;

#[inline]
fn lerp_pt(a: &StrokePoint, b: &StrokePoint, f: f64) -> StrokePoint {
    let ff = f as f32;
    StrokePoint {
        x: a.x + (b.x - a.x) * f,
        y: a.y + (b.y - a.y) * f,
        pressure: a.pressure + (b.pressure - a.pressure) * ff,
        tilt_x: a.tilt_x + (b.tilt_x - a.tilt_x) * ff,
        tilt_y: a.tilt_y + (b.tilt_y - a.tilt_y) * ff,
        rotation: a.rotation + (b.rotation - a.rotation) * ff,
        wheel: a.wheel + (b.wheel - a.wheel) * ff,
        time: a.time + (b.time - a.time) * f,
    }
}

// ---------------------------------------------------------------------------
// Smoothing
// ---------------------------------------------------------------------------

/// Incremental stroke smoothing: exponential (with optional time-based catch-up) or pulled string.
#[derive(Clone, Debug)]
pub struct Smoother {
    cfg: Smoothing,
    zoom: f64,
    pos: Option<StrokePoint>,
    last_input: Option<StrokePoint>,
    /// Stabilizer window (most recent last).
    window: std::collections::VecDeque<StrokePoint>,
}

/// The mean of a window of points (position, and with `sensors` the stylus channels too; the
/// time is the last point's).
fn window_mean(win: &std::collections::VecDeque<StrokePoint>, last: StrokePoint, sensors: bool) -> StrokePoint {
    let n = win.len().max(1) as f64;
    let mut m = StrokePoint { x: 0.0, y: 0.0, pressure: 0.0, tilt_x: 0.0, tilt_y: 0.0, rotation: 0.0, wheel: 0.0, time: last.time };
    for q in win {
        m.x += q.x / n;
        m.y += q.y / n;
        m.pressure += q.pressure / n as f32;
        m.tilt_x += q.tilt_x / n as f32;
        m.tilt_y += q.tilt_y / n as f32;
        m.rotation += q.rotation / n as f32;
        m.wheel += q.wheel / n as f32;
    }
    if !sensors {
        m = StrokePoint { x: m.x, y: m.y, ..last };
    }
    m
}

impl Smoother {
    /// `zoom` is the view zoom (only used with `adjust_for_zoom`).
    pub fn new(cfg: &Smoothing, zoom: f32) -> Self {
        Self { cfg: cfg.clone(), zoom: f64::from(zoom.max(0.01)), pos: None, last_input: None, window: std::collections::VecDeque::new() }
    }

    fn active(&self) -> bool {
        self.cfg.enabled && (self.cfg.amount > 0.0 || self.stabilizing())
    }

    fn stabilizing(&self) -> bool {
        self.cfg.enabled && self.cfg.stabilizer
    }

    /// Smooth Pressure: the output point's pressure follows the input with the same lag as its
    /// position (the exponential factor `step`).
    fn smooth_pressure(&self, prev: &StrokePoint, p: &StrokePoint, step: f64) -> f32 {
        if self.cfg.smooth_pressure { prev.pressure + (p.pressure - prev.pressure) * step as f32 } else { p.pressure }
    }

    /// Stabilizer: how many of the latest points to average at this speed (px/ms, screen space).
    fn sample_count(&self, speed: f64) -> usize {
        let slow = self.cfg.samples_slow.clamp(1, MAX_STABILIZER_SAMPLES as u32) as f64;
        let fast = self.cfg.samples_fast.clamp(1, MAX_STABILIZER_SAMPLES as u32) as f64;
        let t = if speed.is_finite() { (speed / f64::from(VELOCITY_FULL_PX_PER_MS)).clamp(0.0, 1.0) } else { 0.0 };
        (slow + (fast - slow) * t).round().clamp(1.0, MAX_STABILIZER_SAMPLES as f64) as usize
    }

    fn push_stabilized(&mut self, prev_input: Option<StrokePoint>, p: StrokePoint, out: &mut Vec<StrokePoint>) {
        let Some(pos) = self.pos else {
            self.pos = Some(p);
            self.window.clear();
            self.window.push_back(p);
            out.push(p);
            return;
        };
        // Dead zone: the pointer has to leave `delay` screen pixels around the brush.
        let mut delay = f64::from(self.cfg.delay.max(0.0));
        if self.cfg.adjust_for_zoom {
            delay /= self.zoom;
        }
        if delay > 0.0 && (p.x - pos.x).hypot(p.y - pos.y) <= delay {
            return;
        }
        let speed = match prev_input {
            Some(q) if p.time > q.time && (p.time - q.time).is_finite() => (p.x - q.x).hypot(p.y - q.y) * self.zoom / (p.time - q.time),
            _ => 0.0,
        };
        let n = self.sample_count(speed);
        self.window.push_back(p);
        while self.window.len() > n {
            self.window.pop_front();
        }
        let np = window_mean(&self.window, p, self.cfg.stabilize_sensors);
        self.pos = Some(np);
        out.push(np);
    }

    /// Feed one input point; returns the brush positions to paint through.
    pub fn push(&mut self, p: StrokePoint, out: &mut Vec<StrokePoint>) {
        let prev_input = self.last_input.replace(p);
        if !self.active() {
            out.push(p);
            return;
        }
        if self.stabilizing() {
            self.push_stabilized(prev_input, p, out);
            return;
        }
        let Some(pos) = self.pos else {
            self.pos = Some(p);
            out.push(p);
            return;
        };
        let amount = f64::from(self.cfg.amount.clamp(0.0, 1.0));
        if self.cfg.pulled_string {
            let mut len = amount * MAX_STRING_PX;
            if self.cfg.adjust_for_zoom {
                len /= self.zoom;
            }
            let (dx, dy) = (p.x - pos.x, p.y - pos.y);
            let d = (dx * dx + dy * dy).sqrt();
            if d > len {
                let k = (d - len) / d;
                let np = StrokePoint { x: pos.x + dx * k, y: pos.y + dy * k, pressure: self.smooth_pressure(&pos, &p, k), ..p };
                self.pos = Some(np);
                out.push(np);
            }
            return;
        }
        // Exponential: every SMOOTHING_STEP_MS of stroke time moves `a` of the way towards the
        // pointer, so the feel doesn't depend on how often the device reports. Input without
        // timestamps takes one step per point.
        let a = 1.0 - amount.min(0.95);
        let dt = prev_input.map_or(0.0, |q| p.time - q.time);
        let still = prev_input.is_some_and(|q| q.x == p.x && q.y == p.y);
        if still && !self.cfg.catch_up {
            // Without Stroke Catch-Up the brush stops while the pointer pauses (Photoshop).
            return;
        }
        let (iters, step) = if dt > 0.0 && dt.is_finite() {
            let steps = dt / SMOOTHING_STEP_MS;
            let iters = steps.ceil().clamp(1.0, 64.0);
            // Each of `iters` moves covers `steps / iters` steps' worth of pull.
            (iters as usize, 1.0 - (1.0 - a).powf(steps.min(64.0) / iters))
        } else {
            (1, a)
        };
        let mut cur = pos;
        for _ in 0..iters {
            cur = StrokePoint { x: cur.x + (p.x - cur.x) * step, y: cur.y + (p.y - cur.y) * step, pressure: self.smooth_pressure(&cur, &p, step), ..p };
            out.push(cur);
        }
        self.pos = Some(cur);
    }

    /// Is Stroke Catch-Up still pulling the brush towards a pointer that has stopped? Then the
    /// stroke needs time to keep passing (repeats of the last point) while the pointer is held.
    pub fn lagging(&self) -> bool {
        match (self.active() && self.cfg.catch_up && !self.cfg.pulled_string && !self.stabilizing(), self.pos, self.last_input) {
            (true, Some(pos), Some(last)) => (pos.x - last.x).abs() + (pos.y - last.y).abs() > 0.01,
            _ => false,
        }
    }

    /// The string between the brush and the pointer: `(brush, pointer)` document positions while
    /// smoothing holds the brush back from the last input point, `None` when they coincide (or
    /// smoothing is off). The canvas draws it, as Photoshop does for Pulled String Mode.
    pub fn string(&self) -> Option<([f64; 2], [f64; 2])> {
        let (pos, last) = (self.pos?, self.last_input?);
        (self.active() && (pos.x != last.x || pos.y != last.y)).then_some(([pos.x, pos.y], [last.x, last.y]))
    }

    /// End of stroke: with Catch-Up On Stroke End the brush finishes at the last pointer position.
    pub fn finish(&mut self, out: &mut Vec<StrokePoint>) {
        if !self.active() {
            return;
        }
        if self.stabilizing() {
            // Finish Line: the stroke runs on to the last pointer position; otherwise it stops
            // where the average left it.
            if self.cfg.finish_line
                && let (Some(pos), Some(last)) = (self.pos, self.last_input)
                && (pos.x != last.x || pos.y != last.y)
            {
                self.pos = Some(last);
                out.push(last);
            }
            return;
        }
        if !self.cfg.catch_up_on_end {
            return;
        }
        if let (Some(pos), Some(last)) = (self.pos, self.last_input)
            && (pos.x != last.x || pos.y != last.y)
        {
            self.pos = Some(last);
            out.push(last);
        }
    }
}

/// Smooth a whole point list with the brush's smoothing settings.
pub fn smooth_points(cfg: &Smoothing, zoom: f32, pts: &[StrokePoint]) -> Vec<StrokePoint> {
    let mut s = Smoother::new(cfg, zoom);
    let mut out = Vec::with_capacity(pts.len());
    for p in pts {
        s.push(*p, &mut out);
    }
    s.finish(&mut out);
    out
}

// ---------------------------------------------------------------------------
// Path walking
// ---------------------------------------------------------------------------

/// One spacing (or airbrush time) step along the path.
#[derive(Clone, Copy, Debug)]
pub struct StepInput {
    pub point: StrokePoint,
    /// Visual (counter-clockwise, y-up) direction of travel in radians.
    pub direction: f32,
    pub initial_direction: f32,
    /// Sequential step number (0 = first).
    pub step: u64,
    /// Pointer speed over the segment this step lies on, in pixels per millisecond of stroke
    /// time; 0 when the input carries no timestamps.
    pub speed: f32,
}

impl StepInput {
    /// A step at `point` with no direction or speed (for spacing lookups before the path exists).
    pub fn at(point: StrokePoint) -> Self {
        Self { point, direction: 0.0, initial_direction: 0.0, step: 0, speed: 0.0 }
    }
}

/// Walks a polyline and emits steps every `step_len(point)` pixels, plus airbrush steps in time.
#[derive(Clone, Debug)]
pub struct PathWalker {
    last: Option<StrokePoint>,
    /// Path distance from `last` to the next step.
    next_at: f64,
    /// First point, emitted lazily once the initial direction is known.
    pending_first: Option<StrokePoint>,
    initial_dir: Option<f32>,
    dir: f32,
    step: u64,
    /// Speed over the current segment (px/ms), 0 without timestamps.
    speed: f32,
    /// Airbrush interval in ms (None = off).
    interval: Option<f64>,
    /// Airbrush dabs only while the pointer moves (no build-up on a held pointer).
    motion_only: bool,
    time_acc: f64,
    /// Speed spacing (Spacing unchecked): one step per this many ms of stroke time.
    speed_interval: Option<f64>,
    speed_acc: f64,
    /// Pixel-grid stepping for small aliased tips: the dab-centre offset inside a pixel (see
    /// [`PathWalker::pixel_grid`]).
    grid: Option<f64>,
}

impl PathWalker {
    pub fn new(build_up_interval_ms: Option<f64>) -> Self {
        Self {
            last: None,
            next_at: 0.0,
            pending_first: None,
            initial_dir: None,
            dir: 0.0,
            step: 0,
            speed: 0.0,
            interval: build_up_interval_ms,
            motion_only: false,
            time_acc: 0.0,
            speed_interval: None,
            speed_acc: 0.0,
            grid: None,
        }
    }

    /// Step pixel by pixel instead of by distance: one step per pixel along the segment's major
    /// axis, on the line between the pixels holding its ends, each at the pixel's dab centre
    /// (`offset` 0.5 for odd tip diameters, 0 for even ones, as `render::grid_center` snaps). A
    /// small aliased tip (the Pencil) then draws a clean 8-connected staircase: stepping by
    /// distance put two dabs in a row wherever the line crossed a pixel corner (#2139).
    pub fn pixel_grid(mut self, offset: f64) -> Self {
        self.grid = Some(offset);
        self
    }

    /// Emits the pixel steps from `a` to `p` (see [`PathWalker::pixel_grid`]). False, emitting
    /// nothing, when the span is not finite or implausibly long: distance spacing takes over.
    fn grid_steps(&mut self, a: &StrokePoint, p: &StrokePoint, offset: f64, out: &mut impl FnMut(StepInput)) -> bool {
        let cell = |v: f64| if offset > 0.0 { v.floor() } else { v.round() };
        let (x0, y0) = (cell(a.x), cell(a.y));
        let (dx, dy) = (cell(p.x) - x0, cell(p.y) - y0);
        let n = dx.abs().max(dy.abs());
        if !n.is_finite() || n > MAX_GRID_STEPS {
            return false;
        }
        let steps = n as u64;
        for i in 1..=steps {
            let i = i as f64;
            let mut q = lerp_pt(a, p, i / n);
            q.x = x0 + (dx * i / n).round() + offset;
            q.y = y0 + (dy * i / n).round() + offset;
            self.emit(q, out);
        }
        true
    }

    /// Airbrush "motion only": timed dabs are emitted only on segments the pointer moved along.
    pub fn motion_only(mut self, on: bool) -> Self {
        self.motion_only = on;
        self
    }

    /// Space steps by stroke time instead of distance ([`SPEED_SPACING_MS`]): faster movement
    /// spreads them out. Segments without timestamps are spaced by distance.
    pub fn speed_spacing(mut self, interval_ms: f64) -> Self {
        self.speed_interval = Some(interval_ms.max(0.1));
        self
    }

    fn emit(&mut self, point: StrokePoint, out: &mut impl FnMut(StepInput)) {
        let s = StepInput { point, direction: self.dir, initial_direction: self.initial_dir.unwrap_or(self.dir), step: self.step, speed: self.speed };
        self.step += 1;
        out(s);
    }

    pub fn push(&mut self, p: StrokePoint, step_len: &impl Fn(&StrokePoint) -> f64, out: &mut impl FnMut(StepInput)) {
        let Some(a) = self.last else {
            self.last = Some(p);
            self.pending_first = Some(p);
            self.next_at = step_len(&p);
            return;
        };
        let (dx, dy) = (p.x - a.x, p.y - a.y);
        let len = (dx * dx + dy * dy).sqrt();
        if len > 1e-9 {
            self.dir = (-dy).atan2(dx) as f32;
            if self.initial_dir.is_none() {
                self.initial_dir = Some(self.dir);
            }
            let dt = p.time - a.time;
            self.speed = if dt > 0.0 && dt.is_finite() { (len / dt) as f32 } else { 0.0 };
        }
        if let Some(f) = self.pending_first.take() {
            self.emit(f, out);
        }
        if len > 1e-9 {
            // Speed spacing needs the segment's duration; without timestamps (a straight ⇧-click
            // line, or input that carries no times) the segment is spaced by distance instead.
            let dt = p.time - a.time;
            match self.speed_interval.filter(|_| dt > 0.0 && dt.is_finite()) {
                None => {
                    if !self.grid.is_some_and(|g| self.grid_steps(&a, &p, g, out)) {
                        while self.next_at <= len + 1e-9 {
                            let q = lerp_pt(&a, &p, (self.next_at / len).min(1.0));
                            self.emit(q, out);
                            self.next_at += step_len(&q).max(0.25);
                        }
                        self.next_at -= len;
                    }
                }
                Some(iv) => {
                    self.speed_acc += dt;
                    let max_n = (len / MIN_SPEED_STEP).ceil().max(1.0) as usize;
                    let mut n = 0;
                    while self.speed_acc >= iv && n < max_n {
                        self.speed_acc -= iv;
                        let f = ((dt - self.speed_acc) / dt).clamp(0.0, 1.0);
                        self.emit(lerp_pt(&a, &p, f), out);
                        n += 1;
                    }
                    if n == max_n {
                        self.speed_acc = self.speed_acc.rem_euclid(iv);
                    }
                }
            }
        }
        if let Some(iv) = self.interval
            && (!self.motion_only || len > 1e-9)
        {
            let dt = p.time - a.time;
            if dt > 0.0 && dt.is_finite() {
                if dt > iv * MAX_AIRBRUSH_DABS_PER_SEGMENT as f64 {
                    // A stalled or hostile timestamp must not replay an arbitrarily long backlog.
                    self.time_acc = 0.0;
                    self.emit(p, out);
                    self.last = Some(p);
                    return;
                }
                self.time_acc += dt;
                let mut n = 0;
                while self.time_acc >= iv && n < MAX_AIRBRUSH_DABS_PER_SEGMENT {
                    self.time_acc -= iv;
                    let f = ((dt - self.time_acc) / dt).clamp(0.0, 1.0);
                    self.emit(lerp_pt(&a, &p, f), out);
                    n += 1;
                }
            }
        }
        self.last = Some(p);
    }

    pub fn finish(&mut self, out: &mut impl FnMut(StepInput)) {
        if let Some(f) = self.pending_first.take() {
            self.emit(f, out);
        }
    }
}

// ---------------------------------------------------------------------------
// Dynamics
// ---------------------------------------------------------------------------

/// Scalar control value in 0..1. The drawing-angle and Random controls need the dynamic's
/// settings and a random value: see [`sensor_value`]; here they read 1.
pub fn control_value(c: Control, fade_steps: u32, s: &StepInput) -> f32 {
    let p = &s.point;
    match c {
        Control::Off | Control::InitialDirection | Control::Direction | Control::DrawingAngle | Control::Random => 1.0,
        Control::Fade => 1.0 - (s.step as f32 / fade_steps.max(1) as f32).min(1.0),
        Control::PenPressure => p.pressure.clamp(0.0, 1.0),
        // Upright pen = 1, flat pen = 0 (a mouse reports no tilt, so paints at full value).
        Control::PenTilt => 1.0 - (p.tilt_x.hypot(p.tilt_y) / 90.0).clamp(0.0, 1.0),
        Control::StylusWheel => p.wheel.clamp(0.0, 1.0),
        Control::Rotation => (p.rotation.rem_euclid(360.0)) / 360.0,
        Control::Velocity => velocity_value(s.speed),
    }
}

/// Pointer speed as a 0..1 control value (see [`VELOCITY_FULL_PX_PER_MS`]).
#[inline]
pub fn velocity_value(speed: f32) -> f32 {
    if speed.is_finite() { (speed / VELOCITY_FULL_PX_PER_MS).clamp(0.0, 1.0) } else { 0.0 }
}

/// Does a control have a 0..1 input its response curve shapes?
pub fn takes_curve(c: Control) -> bool {
    !matches!(c, Control::Off | Control::InitialDirection | Control::Direction)
}

/// The drawing angle of a step as the sensor reads it, in degrees 0..360: the stroke direction
/// (or the first segment's with `lock`) plus `offset`.
pub fn drawing_angle_deg(d: &Dynamic, s: &StepInput) -> f32 {
    let dir = if d.drawing_angle.lock { s.initial_direction } else { s.direction };
    (dir.to_degrees() + d.drawing_angle.offset).rem_euclid(360.0)
}

/// One sensor's 0..1 value for a dynamic, through `curve`. `rnd` is the dab's random value for
/// [`Control::Random`].
fn sensor_value(d: &Dynamic, c: Control, fade_steps: u32, curve: &crate::curve::ResponseCurve, s: &StepInput, rnd: f32) -> f32 {
    let v = match c {
        Control::DrawingAngle => drawing_angle_deg(d, s) / 360.0,
        Control::Random => rnd.clamp(0.0, 1.0),
        _ => control_value(c, fade_steps, s),
    };
    if takes_curve(c) { curve.eval(v) } else { v }
}

/// A dynamic's control value passed through its own response curve ([`Dynamic::curve`]), combined
/// with its extra sensors by [`Dynamic::combine`]. Controls with no 0..1 input (Off, the direction
/// controls) read 1 and skip the curve. Random sensors read 1 here (no dab index): use
/// [`dynamic_value_rnd`] where a dab's random stream is at hand.
pub fn dynamic_value(d: &Dynamic, s: &StepInput) -> f32 {
    dynamic_value_rnd(d, s, 1.0)
}

/// [`dynamic_value`] with `rnd` as the value of a [`Control::Random`] sensor.
pub fn dynamic_value_rnd(d: &Dynamic, s: &StepInput, rnd: f32) -> f32 {
    let mut acc: Option<f32> = (d.control != Control::Off).then(|| sensor_value(d, d.control, d.fade_steps, &d.curve, s, rnd));
    for (k, sen) in d.sensors.iter().enumerate() {
        if sen.control == Control::Off {
            continue;
        }
        // Each Random sensor draws its own value.
        let r = if k == 0 { rnd } else { (rnd * (k as f32 + 1.0) * 0.618_034).fract() };
        let v = sensor_value(d, sen.control, sen.fade_steps, &sen.curve, s, r);
        acc = Some(match acc {
            None => v,
            Some(a) => match d.combine {
                SensorCombine::Multiply => a * v,
                SensorCombine::Add => (a + v).min(1.0),
                SensorCombine::Max => a.max(v),
                SensorCombine::Min => a.min(v),
                SensorCombine::Difference => (a - v).abs(),
            },
        });
    }
    acc.unwrap_or(1.0).clamp(0.0, 1.0)
}

/// Angle contributed by a control, in degrees (counter-clockwise).
pub fn control_angle(c: Control, fade_steps: u32, s: &StepInput) -> f32 {
    let p = &s.point;
    match c {
        Control::Off => 0.0,
        Control::Fade => 360.0 * (s.step as f32 / fade_steps.max(1) as f32).min(1.0),
        Control::PenPressure => 360.0 * p.pressure.clamp(0.0, 1.0),
        Control::PenTilt => {
            if p.tilt_x == 0.0 && p.tilt_y == 0.0 {
                0.0
            } else {
                (-p.tilt_y).atan2(p.tilt_x).to_degrees()
            }
        }
        Control::StylusWheel => 360.0 * p.wheel.clamp(0.0, 1.0),
        Control::Rotation => p.rotation,
        Control::Velocity => 360.0 * velocity_value(s.speed),
        Control::InitialDirection => s.initial_direction.to_degrees(),
        Control::Direction => s.direction.to_degrees(),
        Control::DrawingAngle => s.direction.to_degrees(),
        Control::Random => 0.0,
    }
}

/// [`control_angle`] for a dynamic: the drawing-angle sensor adds its offset and may lock to the
/// first segment; a Random control turns the tip by a random angle.
pub fn dynamic_angle(d: &Dynamic, s: &StepInput, rnd: f32) -> f32 {
    match d.control {
        Control::DrawingAngle => drawing_angle_deg(d, s),
        Control::Random => 360.0 * rnd.clamp(0.0, 1.0),
        c => control_angle(c, d.fade_steps, s),
    }
}

/// A 0..1 multiplier from a dynamic: control lerps from `minimum` to 1, jitter reduces randomly,
/// never below `minimum`.
fn scalar(d: &Dynamic, s: &StepInput, r: f32) -> f32 {
    if !d.is_active() {
        return 1.0;
    }
    let min = d.minimum.clamp(0.0, 1.0);
    // The jitter draw doubles as the Random sensor's value.
    let cv = dynamic_value_rnd(d, s, r);
    let v = (min + (1.0 - min) * cv) * (1.0 - d.jitter.clamp(0.0, 1.0) * r);
    v.max(min)
}

fn rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let d = mx - mn;
    let h = if d <= 0.0 {
        0.0
    } else if mx == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if mx == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    } / 6.0;
    [h, if mx > 0.0 { d / mx } else { 0.0 }, mx]
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let i = h6.floor();
    let f = h6 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// Colour Dynamics for one dab (or once per stroke, with `index = 0`).
pub fn dynamic_color(b: &BrushSettings, s: &StepInput, seed: u64, index: u64) -> [f32; 4] {
    let cd = &b.color_dynamics;
    let (fg, bg) = (b.color, b.background);
    let r = |st: u64| rand01(seed, index, st);
    let base = if cd.fg_bg.control != Control::Off || cd.fg_bg.has_sensors() { 1.0 - dynamic_value_rnd(&cd.fg_bg, s, r(stream::RANDOM)) } else { 0.0 };
    let t = (base + (1.0 - base) * cd.fg_bg.jitter.clamp(0.0, 1.0) * r(stream::FG_BG)).clamp(0.0, 1.0);
    let rgb = [fg[0] + (bg[0] - fg[0]) * t, fg[1] + (bg[1] - fg[1]) * t, fg[2] + (bg[2] - fg[2]) * t];
    let [mut h, mut sat, mut v] = rgb_to_hsv(rgb);
    h += rand_signed(seed, index, stream::HUE) * cd.hue_jitter.clamp(0.0, 1.0) * 0.5;
    sat = (sat + rand_signed(seed, index, stream::SAT) * cd.saturation_jitter.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    let pu = cd.purity.clamp(-1.0, 1.0);
    sat = if pu >= 0.0 { sat + (1.0 - sat) * pu } else { sat * (1.0 + pu) };
    v = (v + rand_signed(seed, index, stream::BRIGHT) * cd.brightness_jitter.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    let o = hsv_to_rgb(h, sat, v);
    [o[0], o[1], o[2], fg[3] + (bg[3] - fg[3]) * t]
}

/// Base (pre-jitter) diameter at a step: size × size control (pressure etc.). Drives spacing.
pub fn base_diameter(b: &BrushSettings, s: &StepInput) -> f32 {
    let sd = &b.shape_dynamics;
    let mut f = 1.0;
    if sd.enabled && (sd.size.control != Control::Off || sd.size.has_sensors()) {
        let min = sd.size.minimum.clamp(0.0, 1.0);
        f = min + (1.0 - min) * dynamic_value(&sd.size, s);
    } else if b.pressure_size {
        f = s.point.pressure.clamp(0.0, 1.0);
    }
    b.size * f
}

/// Distance to the next dab at a step: the spacing fraction of the base diameter (or the
/// auto-spacing law, `coeff × √diameter` pixels), scaled by the spacing sensor and, without
/// isotropic spacing, by the tip's roundness.
pub fn spacing_px(b: &BrushSettings, s: &StepInput) -> f64 {
    let d = base_diameter(b, s).max(1.0);
    let g = &b.generated;
    let mut step = if g.auto_spacing { g.auto_spacing_coeff.max(0.01) * d.sqrt() } else { d * b.spacing.max(0.01) };
    let sp = &b.spacing_dynamic;
    if sp.is_active() {
        step *= scalar(sp, s, rand01(b.seed, s.step, stream::SPACING)).max(0.01);
    }
    if !b.isotropic_spacing {
        step *= b.roundness.clamp(0.05, 1.0);
    }
    f64::from(step).max(0.5)
}

/// The image hose cell a dab paints with: each dimension picks an index below its rank by its
/// selection mode, and the indices combine row-major (the first dimension slowest). `index` is
/// the dab's position in the stroke (Incremental steps it; Random keys on it).
pub fn hose_cell(h: &ImageHose, s: &StepInput, seed: u64, index: u64) -> u32 {
    let n = h.cells.len() as u64;
    if n == 0 {
        return 0;
    }
    let p = &s.point;
    // A 0..1 value spread over `rank` cells (1 lands in the last one).
    let spread = |v: f32, rank: u64| -> u64 { ((v.clamp(0.0, 1.0) * rank as f32) as u64).min(rank - 1) };
    let single = [crate::brush::HoseDimension { rank: n as u32, selection: HoseSelection::Incremental }];
    let dims: &[crate::brush::HoseDimension] = if h.dims.is_empty() { &single } else { &h.dims };
    let mut cell: u64 = 0;
    for (di, d) in dims.iter().enumerate() {
        let rank = u64::from(d.rank.max(1));
        let i = match d.selection {
            HoseSelection::Incremental => index % rank,
            // Direction is counter-clockwise, y up; the cells go clockwise from a stroke
            // heading right, so a hose drawn for the reference's angular mode turns the same way.
            HoseSelection::Angular => {
                let turn = (-f64::from(s.direction)).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU;
                spread(turn as f32, rank)
            }
            HoseSelection::Random => spread(rand01(seed, index, stream::HOSE + di as u64), rank),
            HoseSelection::Velocity => spread(velocity_value(s.speed), rank),
            HoseSelection::Pressure => spread(p.pressure, rank),
            HoseSelection::TiltX => spread(p.tilt_x / 180.0 + 0.5, rank),
            HoseSelection::TiltY => spread(p.tilt_y / 180.0 + 0.5, rank),
        };
        cell = cell * rank + i;
    }
    (cell % n) as u32
}

/// Builds the primary dabs of a step (Shape Dynamics, Scattering, Transfer, Color, Texture depth).
#[derive(Clone, Debug)]
pub struct DabBuilder {
    pub brush: BrushSettings,
    pub seed: u64,
    /// Per-stroke colour (Color Dynamics with per-tip off), computed at the first step.
    stroke_color: Option<[f32; 4]>,
    next_index: u64,
    /// Direction of the previous step, for Fan Corners.
    last_direction: Option<f32>,
    /// Path length walked so far and the previous step's position, for Colour from gradient.
    travelled: f64,
    last_pos: Option<(f64, f64)>,
}

impl DabBuilder {
    pub fn new(brush: &BrushSettings) -> Self {
        Self { brush: brush.clone(), seed: brush.seed, stroke_color: None, next_index: 0, last_direction: None, travelled: 0.0, last_pos: None }
    }

    /// Colour from gradient: the stop colour at this point of the stroke.
    fn gradient_color(&self) -> Option<[f32; 4]> {
        let b = &self.brush;
        if b.color_gradient.is_empty() {
            return None;
        }
        let len = f64::from(b.color_gradient_length.max(1.0));
        let raw = (self.travelled / len) as f32;
        let t = match b.color_gradient_repeat.as_str() {
            "sawtooth" => raw.rem_euclid(1.0),
            "triangular" => {
                let m = raw.rem_euclid(2.0);
                if m > 1.0 { 2.0 - m } else { m }
            }
            _ => raw.clamp(0.0, 1.0),
        };
        let stops = &b.color_gradient;
        if stops.len() == 1 {
            return Some(stops[0].1);
        }
        if t <= stops[0].0 {
            return Some(stops[0].1);
        }
        for w in stops.windows(2) {
            if t <= w[1].0 {
                let k = if w[1].0 > w[0].0 { (t - w[0].0) / (w[1].0 - w[0].0) } else { 0.0 };
                return Some(std::array::from_fn(|i| w[0].1[i] + (w[1].1[i] - w[0].1[i]) * k));
            }
        }
        stops.last().map(|s| s.1)
    }

    pub fn step_len(&self, p: &StrokePoint) -> f64 {
        spacing_px(&self.brush, &StepInput::at(*p))
    }

    /// The dynamics that read the drawing angle and ask for fanned corners, if any: the smallest
    /// fan threshold in radians.
    fn fan_threshold(&self) -> Option<f32> {
        let b = &self.brush;
        let mut dyns: Vec<&Dynamic> = Vec::new();
        if b.shape_dynamics.enabled {
            dyns.extend([&b.shape_dynamics.size, &b.shape_dynamics.angle, &b.shape_dynamics.roundness]);
        }
        if b.scattering.enabled {
            dyns.extend([&b.scattering.scatter, &b.scattering.count_jitter]);
        }
        if b.transfer.enabled {
            dyns.extend([&b.transfer.opacity, &b.transfer.flow, &b.transfer.wetness, &b.transfer.mix]);
        }
        if b.color_dynamics.enabled {
            dyns.push(&b.color_dynamics.fg_bg);
        }
        if b.texture.enabled && b.texture.each_tip {
            dyns.push(&b.texture.depth_jitter);
        }
        dyns.push(&b.spacing_dynamic);
        dyns.iter()
            .filter(|d| d.drawing_angle.fan_corners && (d.reads_drawing_angle() || d.control == Control::Direction))
            .map(|d| d.drawing_angle.fan_angle.clamp(1.0, 180.0).to_radians())
            .reduce(f32::min)
    }

    /// Build the dabs of a step; at a corner sharper than the fan threshold, extra dabs at the
    /// same point turn the direction through the corner first (Fan Corners).
    pub fn build(&mut self, s: &StepInput, out: &mut Vec<Dab>) {
        if let (Some(th), Some(prev)) = (self.fan_threshold(), self.last_direction) {
            let mut turn = s.direction - prev;
            while turn > std::f32::consts::PI {
                turn -= std::f32::consts::TAU;
            }
            while turn < -std::f32::consts::PI {
                turn += std::f32::consts::TAU;
            }
            if turn.abs() > th && turn.is_finite() {
                let n = ((turn.abs() / th).ceil() as u32).clamp(1, MAX_FAN_DABS);
                for k in 1..n {
                    let dir = prev + turn * (k as f32 / n as f32);
                    self.build_one(&StepInput { direction: dir, ..*s }, out);
                }
            }
        }
        self.last_direction = Some(s.direction);
        let here = (s.point.x, s.point.y);
        if let Some((lx, ly)) = self.last_pos {
            self.travelled += (here.0 - lx).hypot(here.1 - ly);
        }
        self.last_pos = Some(here);
        self.build_one(s, out);
    }

    fn build_one(&mut self, s: &StepInput, out: &mut Vec<Dab>) {
        let b = &self.brush;
        let seed = self.seed;
        let sd = &b.shape_dynamics;
        let sc = &b.scattering;
        let count = if sc.enabled {
            let n = sc.count.clamp(1, 16) as f32;
            let k = scalar(&sc.count_jitter, s, rand01(seed, s.step, stream::COUNT));
            ((n * k).round() as u32).clamp(1, 16)
        } else {
            1
        };
        let base_d = base_diameter(b, s);
        for _ in 0..count {
            let i = self.next_index;
            self.next_index += 1;
            let r = |st: u64| rand01(seed, i, st);
            // Shape.
            let mut diameter = base_d;
            let mut angle = b.angle;
            let mut roundness = b.roundness.clamp(0.01, 1.0);
            let (mut fx, mut fy) = (b.flip_x, b.flip_y);
            if sd.enabled {
                if sd.size.jitter > 0.0 {
                    let min = sd.size.minimum.clamp(0.0, 1.0);
                    let f = (1.0 - sd.size.jitter.clamp(0.0, 1.0) * r(stream::SIZE)).max(min);
                    diameter *= f;
                }
                angle += dynamic_angle(&sd.angle, s, r(stream::RANDOM)) + rand_signed(seed, i, stream::ANGLE) * sd.angle.jitter.clamp(0.0, 1.0) * 180.0;
                // Extra sensors on Angle turn the tip by their share of a full turn.
                if sd.angle.has_sensors() {
                    let mut d = sd.angle;
                    d.control = Control::Off;
                    angle += 360.0 * dynamic_value_rnd(&d, s, r(stream::RANDOM));
                }
                roundness = (roundness * scalar(&sd.roundness, s, r(stream::ROUNDNESS))).clamp(0.01, 1.0);
                if sd.flip_x_jitter && r(stream::FLIP_X) < 0.5 {
                    fx = !fx;
                }
                if sd.flip_y_jitter && r(stream::FLIP_Y) < 0.5 {
                    fy = !fy;
                }
            }
            // Tilt Scale and Brush Projection (Shape Dynamics).
            let (mut proj_angle, mut proj_scale) = (0.0, 1.0);
            if sd.enabled {
                let tilt = s.point.tilt_x.hypot(s.point.tilt_y).clamp(0.0, 90.0);
                if sd.size.control == Control::PenTilt && sd.tilt_scale > 0.0 {
                    let k = 1.0 - sd.tilt_scale.clamp(0.0, 2.0) * 0.5 * tilt / 90.0;
                    roundness = (roundness * k).clamp(0.01, 1.0);
                }
                if sd.brush_projection {
                    angle += s.point.rotation;
                    if tilt > 0.0 {
                        proj_angle = (-s.point.tilt_y).atan2(s.point.tilt_x);
                        proj_scale = tilt.to_radians().cos().max(0.05);
                    }
                }
            }
            // Precision: coarse levels snap the dab size to steps (cheaper tip caches upstream;
            // here it keeps the look of a low-precision preset).
            if matches!(b.tip, crate::brush::TipShape::Round) {
                diameter = b.generated.snap_diameter(diameter);
            }
            let radius = (diameter / 2.0).max(0.5);
            // Scatter.
            let mut c = Point::new(s.point.x, s.point.y);
            if sc.enabled && sc.separate_axes && (sc.along > 0.0 || sc.across > 0.0) {
                let cv = dynamic_value_rnd(&sc.scatter, s, r(stream::RANDOM));
                let along = f64::from(sc.along.clamp(0.0, crate::brush::MAX_SCATTER) * cv * radius);
                let across = f64::from(sc.across.clamp(0.0, crate::brush::MAX_SCATTER) * cv * radius);
                let oa = rand_signed(seed, i, stream::SCATTER_ALONG) as f64 * along;
                let ox = rand_signed(seed, i, stream::SCATTER_X) as f64 * across;
                // Along = the direction of travel, across = perpendicular to it (y-down image space).
                let (dx, dy) = (f64::from(s.direction.cos()), -f64::from(s.direction.sin()));
                c.x += dx * oa - dy * ox;
                c.y += dy * oa + dx * ox;
            } else if sc.enabled && sc.scatter.jitter > 0.0 {
                let cv = dynamic_value_rnd(&sc.scatter, s, r(stream::RANDOM));
                let amt = f64::from(sc.scatter.jitter * cv * radius);
                let (ox, oy) = (rand_signed(seed, i, stream::SCATTER_X) as f64 * amt, rand_signed(seed, i, stream::SCATTER_Y) as f64 * amt);
                if sc.both_axes {
                    c.x += ox;
                    c.y += oy;
                } else {
                    // Perpendicular to the direction of travel (y-down image space).
                    let (dx, dy) = (f64::from(s.direction.cos()), -f64::from(s.direction.sin()));
                    c.x += -dy * ox;
                    c.y += dx * ox;
                }
            }
            // Transfer.
            let tr = &b.transfer;
            let (mut opacity, mut flow) = (1.0, b.flow);
            if tr.enabled {
                opacity = scalar(&tr.opacity, s, r(stream::OPACITY));
                flow *= scalar(&tr.flow, s, r(stream::FLOW));
            }
            // Dynamics presets' Force output: how hard the tip is pressed, as a per-dab flow factor.
            if b.force_dynamic.is_active() {
                flow *= scalar(&b.force_dynamic, s, r(stream::FORCE));
            }
            // Dynamics presets' Hardness output: a per-dab hardness multiplier.
            let hardness = if b.hardness_dynamic.is_active() { scalar(&b.hardness_dynamic, s, r(stream::HARDNESS)).clamp(0.0, 1.0) } else { 1.0 };
            let (wet, mix) = if tr.enabled { (scalar(&tr.wetness, s, r(stream::WET)), scalar(&tr.mix, s, r(stream::MIX))) } else { (1.0, 1.0) };
            if b.pressure_opacity && !(tr.enabled && (tr.opacity.control != Control::Off || tr.opacity.has_sensors())) {
                opacity *= s.point.pressure.clamp(0.0, 1.0);
            }
            // Colour.
            let color = if let Some(c) = self.gradient_color() {
                c
            } else if b.color_dynamics.enabled {
                if b.color_dynamics.per_tip { dynamic_color(b, s, seed, i) } else { *self.stroke_color.get_or_insert_with(|| dynamic_color(b, s, seed, 0)) }
            } else {
                b.color
            };
            // Texture depth (per tip).
            let tx = &b.texture;
            let depth =
                if tx.enabled && tx.each_tip { tx.depth.clamp(0.0, 1.0) * scalar(&tx.depth_jitter, s, r(stream::DEPTH)) } else { tx.depth.clamp(0.0, 1.0) };
            // Image hose: which cell this dab paints with.
            let cell = b.hose.as_ref().filter(|h| h.is_active()).map_or(0, |h| hose_cell(h, s, seed, i));
            out.push(Dab {
                center: c,
                radius,
                alpha: flow.clamp(0.0, 1.0),
                angle: angle.to_radians(),
                roundness,
                flip_x: fx,
                flip_y: fy,
                opacity: opacity.clamp(0.0, 1.0),
                color,
                depth,
                index: i,
                proj_angle,
                proj_scale,
                wet,
                mix,
                cell,
                hardness,
            });
        }
    }
}

/// Builds Dual Brush dabs (the second tip: size, spacing, scatter, count, flip).
#[derive(Clone, Debug)]
pub struct DualBuilder {
    pub brush: BrushSettings,
    next_index: u64,
}

impl DualBuilder {
    pub fn new(brush: &BrushSettings) -> Self {
        Self { brush: brush.clone(), next_index: 0 }
    }
    pub fn step_len(&self, _p: &StrokePoint) -> f64 {
        let d = &self.brush.dual_brush;
        f64::from(d.size.max(1.0) * d.spacing.max(0.01)).max(0.5)
    }
    pub fn build(&mut self, s: &StepInput, out: &mut Vec<Dab>) {
        let d = &self.brush.dual_brush;
        let seed = self.brush.seed ^ 0xD0A1;
        for _ in 0..d.count.clamp(1, 16) {
            let i = self.next_index;
            self.next_index += 1;
            let radius = (d.size / 2.0).max(0.5);
            let amt = f64::from(d.scatter * radius);
            let (ox, oy) = (rand_signed(seed, i, stream::DUAL_X) as f64 * amt, rand_signed(seed, i, stream::DUAL_Y) as f64 * amt);
            let mut c = Point::new(s.point.x, s.point.y);
            if d.both_axes {
                c.x += ox;
                c.y += oy;
            } else {
                let (dx, dy) = (f64::from(s.direction.cos()), -f64::from(s.direction.sin()));
                c.x += -dy * ox;
                c.y += dx * ox;
            }
            let flip = d.flip && rand01(seed, i, stream::DUAL_FLIP) < 0.5;
            out.push(Dab {
                center: c,
                radius,
                alpha: 1.0,
                angle: d.angle.to_radians(),
                roundness: d.roundness.clamp(0.01, 1.0),
                flip_x: flip,
                flip_y: false,
                opacity: 1.0,
                color: [0.0, 0.0, 0.0, 1.0],
                depth: 1.0,
                index: i,
                proj_angle: 0.0,
                proj_scale: 1.0,
                wet: 1.0,
                mix: 1.0,
                cell: 0,
                hardness: 1.0,
            });
        }
    }
}

/// Apply Brush Pose to an input point.
pub fn apply_pose(b: &BrushSettings, mut p: StrokePoint) -> StrokePoint {
    let pose = &b.pose;
    if pose.enabled {
        if pose.override_tilt {
            p.tilt_x = pose.tilt_x;
            p.tilt_y = pose.tilt_y;
        }
        if pose.override_rotation {
            p.rotation = pose.rotation;
        }
        if pose.override_pressure {
            p.pressure = pose.pressure;
        }
    }
    p
}

/// Incremental dab generation for a whole brush (primary and optional dual dabs).
#[derive(Clone, Debug)]
pub struct DabGenerator {
    smoother: Smoother,
    walker: PathWalker,
    dual_walker: Option<PathWalker>,
    builder: DabBuilder,
    dual: Option<DualBuilder>,
    scratch: Vec<StrokePoint>,
}

impl DabGenerator {
    /// Does the stroke change with time while the pointer is held still (airbrush Build-up, or
    /// smoothing still catching up)? The canvas then keeps feeding it the held point.
    pub fn wants_time(&self) -> bool {
        (self.walker.interval.is_some() && !self.walker.motion_only) || self.smoother.lagging()
    }

    /// See [`Smoother::string`].
    pub fn smoothing_string(&self) -> Option<([f64; 2], [f64; 2])> {
        self.smoother.string()
    }

    pub fn new(brush: &BrushSettings, zoom: f32) -> Self {
        let brush = brush.bounded_for_render();
        let interval = brush.build_up.then(|| 1000.0 / f64::from(brush.build_up_rate.clamp(0.1, 1000.0)));
        let dual_on = brush.dual_brush.enabled;
        let mut walker = if brush.spacing_enabled { PathWalker::new(interval) } else { PathWalker::new(interval).speed_spacing(SPEED_SPACING_MS) };
        walker = walker.motion_only(brush.build_up_motion_only);
        // A small aliased tip whose dabs are at most a pixel apart steps pixel by pixel (#2139).
        if brush.aliased && brush.size * brush.spacing.max(0.01) <= 1.0 {
            walker = walker.pixel_grid(f64::from(crate::render::grid_center(0.0, 0.0, brush.size).0));
        }
        Self {
            smoother: Smoother::new(&brush.smoothing, zoom),
            walker,
            dual_walker: dual_on.then(|| PathWalker::new(None)),
            builder: DabBuilder::new(&brush),
            dual: dual_on.then(|| DualBuilder::new(&brush)),
            scratch: Vec::new(),
        }
    }

    fn walk(&mut self, pts: &[StrokePoint], dabs: &mut Vec<Dab>, dual: &mut Vec<Dab>) {
        for p in pts {
            let p = apply_pose(&self.builder.brush, *p);
            // Dynamics presets' Rate output: the airbrush rate follows the sensor at each point.
            if self.builder.brush.build_up && self.builder.brush.rate_dynamic.is_active() {
                let b = &self.builder.brush;
                let k = scalar(&b.rate_dynamic, &StepInput::at(p), rand01(b.seed, self.walker.step, stream::RATE)).max(0.01);
                self.walker.interval = Some(1000.0 / f64::from((b.build_up_rate * k).clamp(0.1, 1000.0)));
            }
            let b = &self.builder;
            let len = |q: &StrokePoint| b.step_len(q);
            let mut steps = Vec::new();
            self.walker.push(p, &len, &mut |s| steps.push(s));
            for s in &steps {
                self.builder.build(s, dabs);
            }
            if let (Some(w), Some(db)) = (self.dual_walker.as_mut(), self.dual.as_mut()) {
                let mut steps = Vec::new();
                let len = |q: &StrokePoint| db.step_len(q);
                w.push(p, &len, &mut |s| steps.push(s));
                for s in &steps {
                    db.build(s, dual);
                }
            }
        }
    }

    /// Feed input points; appends the new primary and dual dabs.
    pub fn push(&mut self, pts: &[StrokePoint], dabs: &mut Vec<Dab>, dual: &mut Vec<Dab>) {
        let mut sm = std::mem::take(&mut self.scratch);
        sm.clear();
        for p in pts {
            self.smoother.push(*p, &mut sm);
        }
        self.walk(&sm, dabs, dual);
        self.scratch = sm;
    }

    /// End of stroke.
    pub fn finish(&mut self, dabs: &mut Vec<Dab>, dual: &mut Vec<Dab>) {
        let mut sm = Vec::new();
        self.smoother.finish(&mut sm);
        self.walk(&sm, dabs, dual);
        let mut steps = Vec::new();
        self.walker.finish(&mut |s| steps.push(s));
        for s in &steps {
            self.builder.build(s, dabs);
        }
        if let (Some(w), Some(db)) = (self.dual_walker.as_mut(), self.dual.as_mut()) {
            let mut steps = Vec::new();
            w.finish(&mut |s| steps.push(s));
            for s in &steps {
                db.build(s, dual);
            }
        }
    }
}
