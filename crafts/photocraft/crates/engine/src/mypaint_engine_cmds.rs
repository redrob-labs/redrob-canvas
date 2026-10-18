//! MyPaint-style brush engine (`paint.mypaint`): paints with brush presets of the MyPaint `.myb`
//! format ([`redrob_photo_io::myb`]) interpreted here: dabs placed along the stroke every
//! `radius / dabs_per_actual_radius` (and per second), each a round or elliptical dab with the
//! preset's hardness and opacity, driven by the pressure, speed, random, stroke-progress,
//! direction and tilt inputs through the preset's curves; colour shifts (`color_h/s/v`,
//! `change_color_*`), random offsets, radius jitter, eraser and lock-alpha settings apply.
//! Smudge and the custom input are not simulated. `mypaint.list` and `mypaint.import` keep the
//! preset library (the built-ins plus imported `.myb` files).

use redrob_photo_geom::Rect;
use redrob_photo_io::myb::{MybBrush, MybSetting};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::ink_cmds::InkPoint;
use crate::{EngineError, Result, Session};

const CMD: &str = "paint.mypaint";
const MAX_DABS: usize = 400_000;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// Every preset: the built-ins, then the imported ones (an import with a built-in's name wins).
pub fn brushes(s: &Session) -> Vec<MybBrush> {
    let mut out = redrob_photo_io::myb::builtins();
    for b in &s.mypaint_imported {
        match out.iter_mut().find(|x| x.name == b.name) {
            Some(x) => *x = b.clone(),
            None => out.push(b.clone()),
        }
    }
    out
}

fn find_brush(s: &Session, name: &str) -> Option<MybBrush> {
    brushes(s).into_iter().find(|b| b.name == name)
}

/// Parses `points` like the Ink tool's (`[x, y, pressure?, tiltX?, tiltY?, rotation?, timeMs?]`).
fn parse_points(p: &Value) -> Result<Vec<InkPoint>> {
    let a = p.get("points").and_then(Value::as_array).ok_or_else(|| bad(CMD, "pass `points`: [[x, y, pressure?, tiltX?, tiltY?, rotation?, timeMs?], ...]"))?;
    if a.len() > 100_000 {
        return Err(bad(CMD, "too many points"));
    }
    let mut out = Vec::with_capacity(a.len());
    for (i, v) in a.iter().enumerate() {
        let n: Vec<f64> = v.as_array().map(|q| q.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
        if n.len() < 2 || n.iter().any(|x| !x.is_finite()) {
            return Err(bad(CMD, format!("point {i} must be [x, y, ...] finite numbers")));
        }
        let pressure = n.get(2).copied().unwrap_or(1.0).clamp(0.0, 1.0);
        let (tilt_x, tilt_y) = if n.len() >= 6 { (n[3], n[4]) } else { (0.0, 0.0) };
        let time_ms = match n.len() {
            4 => n[3],
            7 => n[6],
            _ => i as f64 * 10.0,
        };
        out.push(InkPoint { x: n[0], y: n[1], pressure, tilt_x, tilt_y, time_ms });
    }
    if out.is_empty() {
        return Err(bad(CMD, "`points` is empty"));
    }
    Ok(out)
}

/// A tiny deterministic generator (xorshift) so a replayed stroke paints the same pixels.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        ((x >> 40) as f32) / ((1u64 << 24) as f32)
    }
}

/// The per-dab inputs the curves read.
struct Inputs {
    pressure: f32,
    speed1: f32,
    speed2: f32,
    random: f32,
    stroke: f32,
    direction: f32,
    tilt_declination: f32,
    tilt_ascension: f32,
}

impl Inputs {
    fn get(&self, name: &str) -> f32 {
        match name {
            "pressure" => self.pressure,
            "speed1" => self.speed1,
            "speed2" => self.speed2,
            "random" => self.random,
            "stroke" => self.stroke,
            "direction" => self.direction,
            "tilt_declination" => self.tilt_declination,
            "tilt_ascension" => self.tilt_ascension,
            _ => 0.0,
        }
    }
}

fn eval(set: &MybSetting, inp: &Inputs) -> f32 {
    set.eval(&|k| inp.get(k))
}

fn rgb_to_hsv([r, g, b]: [f32; 3]) -> [f32; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= 1e-6 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    let s = if max <= 1e-6 { 0.0 } else { d / max };
    [h, s, max]
}

fn hsv_to_rgb([h, s, v]: [f32; 3]) -> [f32; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h.floor();
    let f = h - i;
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

/// Options the tool passes on top of the preset.
struct Overrides {
    radius_log: f32,
    opaque: f32,
    hardness: Option<f32>,
    erase: bool,
}

/// One dab to draw.
struct Dab {
    x: f32,
    y: f32,
    radius: f32,
    hardness: f32,
    opaque: f32,
    color: [f32; 3],
    ratio: f32,
    angle: f32,
    erase: bool,
    aa: bool,
}

/// Lays the dabs of a stroke out.
fn dabs(brush: &MybBrush, pts: &[InkPoint], fg: [f32; 3], o: &Overrides, seed: u64) -> Vec<Dab> {
    let mut rng = Rng(seed | 1);
    let s_radius = brush.setting("radius_logarithmic");
    let s_opaque = brush.setting("opaque");
    let s_opaque_mul = brush.settings.get("opaque_multiply").cloned();
    let s_hardness = brush.setting("hardness");
    let s_dpar = brush.setting("dabs_per_actual_radius");
    let s_dpbr = brush.setting("dabs_per_basic_radius");
    let s_dps = brush.setting("dabs_per_second");
    let s_off_random = brush.setting("offset_by_random");
    let s_rad_random = brush.setting("radius_by_random");
    let s_ratio = brush.setting("elliptical_dab_ratio");
    let s_angle = brush.setting("elliptical_dab_angle");
    let s_eraser = brush.setting("eraser");
    let s_aa = brush.base("anti_aliasing") > 0.5;
    let color_h = brush.setting("color_h");
    let color_s = brush.setting("color_s");
    let color_v = brush.setting("color_v");
    let change_h = brush.setting("change_color_h");
    let change_s = brush.setting("change_color_hsv_s");
    let change_v = brush.setting("change_color_v");
    let pressure_gain = brush.base("pressure_gain_log").exp();
    let total_len: f64 = pts.windows(2).map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y)).sum();
    let t0 = pts.first().map_or(0.0, |p| p.time_ms);
    let duration_ms = brush.base("stroke_duration_logarithmic").exp() * 1000.0 / 4.0;
    let base_hsv = rgb_to_hsv(fg);
    let mut out = Vec::new();
    let mut dist_since = f32::INFINITY;
    let mut walked = 0.0f64;
    let mut speed1 = 0.0f32;
    let mut speed2 = 0.0f32;
    let emit = |pt: &InkPoint, prev: Option<&InkPoint>, walked: f64, speed1: f32, speed2: f32, out: &mut Vec<Dab>, rng: &mut Rng| -> f32 {
        let dir = prev.map_or(0.0, |q| ((pt.y - q.y).atan2(pt.x - q.x).to_degrees() as f32).rem_euclid(180.0));
        let tilt_decl = ((pt.tilt_x.hypot(pt.tilt_y)) as f32).min(90.0);
        let tilt_asc = (pt.tilt_y.atan2(pt.tilt_x).to_degrees()) as f32;
        let inp = Inputs {
            pressure: (pt.pressure as f32 * pressure_gain).clamp(0.0, 1.0),
            speed1,
            speed2,
            random: rng.next(),
            stroke: if duration_ms > 0.0 { (((pt.time_ms - t0) as f32) / duration_ms).clamp(0.0, 1.0) } else { (walked / total_len.max(1.0)) as f32 },
            direction: dir,
            tilt_declination: tilt_decl,
            tilt_ascension: tilt_asc,
        };
        let mut radius_log = eval(&s_radius, &inp) + o.radius_log;
        let rr = eval(&s_rad_random, &inp);
        if rr > 0.0 {
            radius_log += (rng.next() - 0.5) * 2.0 * rr * 0.5;
        }
        let radius = radius_log.exp().clamp(0.3, 500.0);
        let mut opaque = (eval(&s_opaque, &inp) * o.opaque).clamp(0.0, 1.0);
        if let Some(m) = &s_opaque_mul {
            opaque *= eval(m, &inp).clamp(0.0, 1.0);
        }
        let hardness = o.hardness.unwrap_or_else(|| eval(&s_hardness, &inp)).clamp(0.01, 1.0);
        let off = eval(&s_off_random, &inp) * radius;
        let (dx, dy) = if off > 0.0 { ((rng.next() - 0.5) * 2.0 * off, (rng.next() - 0.5) * 2.0 * off) } else { (0.0, 0.0) };
        let hsv = [
            base_hsv[0] + eval(&color_h, &inp) + eval(&change_h, &inp) * 0.5,
            (base_hsv[1] + eval(&color_s, &inp) + eval(&change_s, &inp)).clamp(0.0, 1.0),
            (base_hsv[2] + eval(&color_v, &inp) + eval(&change_v, &inp)).clamp(0.0, 1.0),
        ];
        out.push(Dab {
            x: pt.x as f32 + dx,
            y: pt.y as f32 + dy,
            radius,
            hardness,
            opaque,
            color: hsv_to_rgb(hsv),
            ratio: eval(&s_ratio, &inp).max(1.0),
            angle: eval(&s_angle, &inp),
            erase: o.erase || eval(&s_eraser, &inp) > 0.5,
            aa: s_aa,
        });
        // The spacing to the next dab.
        let dpar = eval(&s_dpar, &inp).max(0.0);
        let dpbr = eval(&s_dpbr, &inp).max(0.0);
        let per_px = (dpar / radius) + (dpbr / s_radius.base.exp().max(0.3));
        if per_px > 0.0 { 1.0 / per_px } else { radius * 0.5 }
    };
    let mut spacing = emit(&pts[0], None, 0.0, 0.0, 0.0, &mut out, &mut rng);
    for w in pts.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        let seg = (b.x - a.x).hypot(b.y - a.y);
        let dt = ((b.time_ms - a.time_ms) as f32).max(1.0);
        let v = (seg as f32 / dt * 1000.0).max(0.0); // px/s
        let sp = (v / 100.0 + 1.0).ln();
        speed1 += (sp - speed1) * 0.6;
        speed2 += (sp - speed2) * 0.1;
        let dps = eval(
            &s_dps,
            &Inputs { pressure: b.pressure as f32, speed1, speed2, random: 0.0, stroke: 0.0, direction: 0.0, tilt_declination: 0.0, tilt_ascension: 0.0 },
        );
        let time_spacing = if dps > 0.0 { v / dps } else { f32::INFINITY };
        let step = spacing.min(time_spacing).max(0.05);
        if seg <= 0.0 {
            continue;
        }
        let mut t = (step - dist_since.min(step)) as f64;
        let mut placed = false;
        while t <= seg {
            let k = t / seg;
            let pt = InkPoint {
                x: a.x + (b.x - a.x) * k,
                y: a.y + (b.y - a.y) * k,
                pressure: a.pressure + (b.pressure - a.pressure) * k,
                tilt_x: a.tilt_x + (b.tilt_x - a.tilt_x) * k,
                tilt_y: a.tilt_y + (b.tilt_y - a.tilt_y) * k,
                time_ms: a.time_ms + (b.time_ms - a.time_ms) * k,
            };
            spacing = emit(&pt, Some(a), walked + t, speed1, speed2, &mut out, &mut rng);
            placed = true;
            t += f64::from(spacing.min(time_spacing).max(0.05));
            if out.len() >= MAX_DABS {
                return out;
            }
        }
        dist_since = if placed { (seg - (t - f64::from(spacing.min(time_spacing).max(0.05)))) as f32 } else { dist_since + seg as f32 };
        walked += seg;
    }
    out
}

/// Draws `dabs` into straight RGBA pixels of `area`.
fn draw(px: &mut [[f32; 4]], area: Rect, dabs: &[Dab], lock_alpha: bool) {
    let w = area.width() as i32;
    let h = area.height() as i32;
    for d in dabs {
        let r = d.radius;
        let (cs, sn) = (d.angle.to_radians().cos(), d.angle.to_radians().sin());
        let reach = r * d.ratio.max(1.0) + 1.0;
        let x0 = ((d.x - reach).floor() as i32 - area.x0).max(0);
        let x1 = ((d.x + reach).ceil() as i32 - area.x0).min(w - 1);
        let y0 = ((d.y - reach).floor() as i32 - area.y0).max(0);
        let y1 = ((d.y + reach).ceil() as i32 - area.y0).min(h - 1);
        if x1 < x0 || y1 < y0 {
            continue;
        }
        let aa_width = if d.aa { (1.0 / r).min(1.0) } else { 0.0 };
        for yy in y0..=y1 {
            for xx in x0..=x1 {
                let fx = (xx + area.x0) as f32 + 0.5 - d.x;
                let fy = (yy + area.y0) as f32 + 0.5 - d.y;
                // Into the dab's frame: the ellipse's long axis along `angle`.
                let u = fx * cs + fy * sn;
                let v = -fx * sn + fy * cs;
                let dist = ((u / d.ratio).powi(2) + v * v).sqrt() / r;
                if dist >= 1.0 + aa_width {
                    continue;
                }
                // The dab's fall-off: full inside the hardness radius, then a linear ramp.
                let mut opa = if dist < d.hardness { dist + 1.0 - dist / d.hardness } else { d.hardness / (1.0 - d.hardness).max(1e-4) * (1.0 - dist) };
                opa = opa.clamp(0.0, 1.0);
                if dist > 1.0 {
                    opa *= ((1.0 + aa_width - dist) / aa_width.max(1e-4)).clamp(0.0, 1.0);
                }
                let a = opa * d.opaque;
                if a <= 0.0 {
                    continue;
                }
                let p = &mut px[(yy * w + xx) as usize];
                if d.erase {
                    p[3] *= 1.0 - a;
                    continue;
                }
                let da = p[3];
                let na = if lock_alpha { da } else { a + da * (1.0 - a) };
                if na <= 0.0 {
                    continue;
                }
                // Straight colour: source over.
                let wa = if lock_alpha { a * da } else { a };
                for (k, pk) in p.iter_mut().enumerate().take(3) {
                    *pk = (d.color[k] * wa + *pk * da * (1.0 - a)) / na.max(1e-6);
                }
                p[3] = na;
            }
        }
    }
}

fn mypaint(s: &mut Session, p: &Value) -> Result<Value> {
    let pts = parse_points(p)?;
    let name = p.get("preset").and_then(Value::as_str).unwrap_or("Soft Round").to_string();
    let brush = find_brush(s, &name).ok_or_else(|| bad(CMD, format!("no MyPaint brush named \"{name}\"")))?;
    let f = |k: &str, d: f32| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(d, |v| v as f32);
    let o = Overrides {
        radius_log: f("radius", 0.0).clamp(-3.0, 3.0),
        opaque: f("opaque", 1.0).clamp(0.0, 2.0),
        hardness: p.get("hardness").and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| (v as f32).clamp(0.01, 1.0)),
        erase: p.get("erase").and_then(Value::as_bool).unwrap_or(false),
    };
    let id = crate::commands::layer_param(s, p)?;
    crate::commands::check_pixels_unlocked(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?;
    let fg = s.tools.foreground;
    let seed = p
        .get("seed")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| pts.iter().fold(0x9E37_79B9u64, |h, q| h.wrapping_mul(31).wrapping_add(q.x.to_bits() ^ q.y.to_bits())));
    let dabs = self::dabs(&brush, &pts, [fg[0], fg[1], fg[2]], &o, seed);
    let lock_alpha = brush.base("lock_alpha") > 0.5;
    let n = dabs.len();
    let mut painted = Rect::new(0, 0, 0, 0);
    s.edit("MyPaint Brush", |doc, _| {
        let canvas = doc.bounds();
        let mut area: Option<Rect> = None;
        for d in &dabs {
            let reach = (d.radius * d.ratio.max(1.0) + 2.0).ceil() as i32;
            let r = Rect::new(d.x.floor() as i32 - reach, d.y.floor() as i32 - reach, d.x.ceil() as i32 + reach, d.y.ceil() as i32 + reach);
            area = Some(area.map_or(r, |a| a.union(&r)));
        }
        let Some(area) = area.map(|a| a.intersect(&canvas)).filter(|a| !a.is_empty()) else { return Ok(()) };
        let locks = doc.effective_locks(id);
        let selection = doc.selection.clone();
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let surf = l.surface_mut().ok_or_else(|| EngineError::Other("the MyPaint brush needs a pixel layer".into()))?;
        let mut px = vec![[0.0f32; 4]; (area.width() as usize) * (area.height() as usize)];
        surf.read_rgba_into(area, &mut px);
        let before = px.clone();
        draw(&mut px, area, &dabs, lock_alpha || locks.transparency);
        if let Some(sel) = selection.as_ref() {
            let mask = redrob_photo_algo::selection::mask_from_surface(Some(sel), area);
            for ((p, b), m) in px.iter_mut().zip(&before).zip(&mask) {
                for k in 0..4 {
                    p[k] = b[k] + (p[k] - b[k]) * m;
                }
            }
        }
        let fmt = surf.format();
        let ch = fmt.channels();
        let mut vals = Vec::with_capacity(px.len() * ch);
        let mut tmp = vec![0.0f32; ch];
        for p in &px {
            redrob_photo_raster::from_rgba_into(&fmt, *p, &mut tmp);
            vals.extend_from_slice(&tmp);
        }
        surf.write_region(area, &vals);
        surf.prune();
        painted = area;
        Ok(())
    })?;
    Ok(json!({ "preset": name, "dabs": n, "bounds": [painted.x0, painted.y0, painted.width(), painted.height()] }))
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let items: Vec<Value> = brushes(s)
        .iter()
        .map(|b| {
            json!({
                "name": b.name, "group": b.group, "description": b.description,
                "radius": b.base("radius_logarithmic").exp(), "hardness": b.base("hardness"), "opaque": b.base("opaque"),
                "eraser": b.base("eraser") > 0.5,
                "builtin": !s.mypaint_imported.iter().any(|x| x.name == b.name),
            })
        })
        .collect();
    Ok(json!({ "brushes": items }))
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const IMPORT: &str = "mypaint.import";
    let (bytes, stem) = crate::preset_import_cmds::file_bytes(p, IMPORT, "a .myb brush file")?;
    let fallback = p.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| if stem.is_empty() { "Imported".to_string() } else { stem });
    let mut brush = redrob_photo_io::myb::parse(&bytes, &fallback).map_err(|e| EngineError::Other(e.to_string()))?;
    if let Some(n) = p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()) {
        brush.name = n.to_string();
    }
    if s.mypaint_imported.len() >= 512 && !s.mypaint_imported.iter().any(|b| b.name == brush.name) {
        return Err(EngineError::Other("at most 512 imported MyPaint brushes".into()));
    }
    let name = brush.name.clone();
    match s.mypaint_imported.iter_mut().find(|b| b.name == name) {
        Some(b) => *b = brush,
        None => s.mypaint_imported.push(brush),
    }
    Ok(json!({ "name": name }))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let name = p.get("name").and_then(Value::as_str).ok_or_else(|| bad("mypaint.remove", "pass `name`"))?;
    let before = s.mypaint_imported.len();
    s.mypaint_imported.retain(|b| b.name != name);
    if s.mypaint_imported.len() == before {
        return Err(bad("mypaint.remove", format!("`{name}` is not an imported brush")));
    }
    Ok(json!({ "removed": name }))
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const EXPORT: &str = "mypaint.export";
    let name = p.get("name").and_then(Value::as_str).ok_or_else(|| bad(EXPORT, "pass `name`"))?;
    let path = p.get("path").and_then(Value::as_str).filter(|v| !v.trim().is_empty()).ok_or_else(|| bad(EXPORT, "pass `path` (the .myb to write)"))?;
    let brush = find_brush(s, name).ok_or_else(|| bad(EXPORT, format!("no MyPaint brush named \"{name}\"")))?;
    let text = redrob_photo_io::myb::write(&brush);
    crate::file_cmds::write_file(path, text.as_bytes())?;
    Ok(json!({ "name": name, "path": path }))
}

fn has_pixels(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let id = d.active_layer.ok_or("no active layer")?;
    if d.doc.layer(id).and_then(redrob_photo_doc::Layer::surface).is_some() { Ok(()) } else { Err("the active layer has no pixels".into()) }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: CMD,
            label: "MyPaint Brush",
            menu: &[],
            shortcut: None,
            params: r##"{"points":[[x,y,pressure?,tiltX?,tiltY?,rotation?,timeMs?],...],"preset":name="Soft Round","radius":log offset -3..3=0,"opaque":0-2=1,"hardness":0.01-1?,"erase":bool=false,"seed":u64?,"layer":id?} → {preset, dabs, bounds}"##,
            enabled: has_pixels,
            run: mypaint,
            journal: true,
        },
        CommandSpec {
            id: "mypaint.list",
            label: "List MyPaint Brushes",
            menu: &[],
            shortcut: None,
            params: r##"{} → {brushes: [{name, group, description, radius, hardness, opaque, eraser, builtin}]}"##,
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "mypaint.import",
            label: "Import MyPaint Brush…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".myb file"?,"data":base64?,"name":string?} → {name}"##,
            enabled: always,
            run: import,
            journal: false,
        },
        CommandSpec {
            id: "mypaint.remove",
            label: "Remove MyPaint Brush",
            menu: &[],
            shortcut: None,
            params: r##"{"name":string} → {removed}"##,
            enabled: always,
            run: remove,
            journal: false,
        },
        CommandSpec {
            id: "mypaint.export",
            label: "Export MyPaint Brush…",
            menu: &[],
            shortcut: None,
            params: r##"{"name":string,"path":".myb to write"} → {name, path}"##,
            enabled: always,
            run: export,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strokes_paint_and_presets_list() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 80, "height": 40})).unwrap_or_else(|e| panic!("{e}"));
        let r = s
            .execute(CMD, json!({"points": [[10, 20, 0.8, 0.0, 0.0, 0.0, 0.0], [70, 20, 0.8, 0.0, 0.0, 0.0, 200.0]], "preset": "Ink Pen", "seed": 7}))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(r["dabs"].as_u64().unwrap_or(0) > 5, "{r}");
        let l = s.execute("mypaint.list", json!({})).unwrap_or_else(|e| panic!("{e}"));
        assert!(l["brushes"].as_array().is_some_and(|a| a.len() >= 7));
        let myb = redrob_photo_io::myb::write(&redrob_photo_io::myb::builtins()[0]);
        let data = redrob_photo_paint::tile::b64_encode(myb.as_bytes());
        let i = s.execute("mypaint.import", json!({"data": data, "name": "Mine"})).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(i["name"], "Mine");
        assert!(find_brush(&s, "Mine").is_some());
        s.execute("mypaint.remove", json!({"name": "Mine"})).unwrap_or_else(|e| panic!("{e}"));
        assert!(find_brush(&s, "Mine").is_none());
    }
}
