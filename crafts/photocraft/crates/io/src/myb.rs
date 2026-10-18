//! MyPaint brush files (`.myb`): JSON (format version 2 and 3) with a `settings` object mapping
//! each setting (`opaque`, `radius_logarithmic`, `hardness`, ...) to a `base_value` and `inputs`
//! (`pressure`, `speed1`, `speed2`, `random`, `stroke`, `direction`, `tilt_declination`,
//! `tilt_ascension`, `custom`...) each a list of `[input, output]` points, piecewise linear. The
//! old text form (version 1: `name value | input (x, y), (x, y)` lines) is read too. Written
//! from the public format description; the companion `.myb.png` preview is optional.

use std::collections::BTreeMap;

use crate::IoError;

/// One setting: its base value plus the input curves added to it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MybSetting {
    pub base: f32,
    /// input name → control points (sorted by input value).
    pub inputs: BTreeMap<String, Vec<[f32; 2]>>,
}

impl MybSetting {
    pub fn constant(v: f32) -> Self {
        Self { base: v, inputs: BTreeMap::new() }
    }

    /// The setting's value for the given inputs (base plus every curve's output, as the format
    /// defines: contributions add).
    pub fn eval(&self, input: &dyn Fn(&str) -> f32) -> f32 {
        let mut v = self.base;
        for (name, pts) in &self.inputs {
            v += interp(pts, input(name));
        }
        v
    }
}

/// Piecewise-linear interpolation through `pts` (clamped at the ends).
pub fn interp(pts: &[[f32; 2]], x: f32) -> f32 {
    match pts {
        [] => 0.0,
        [p] => p[1],
        _ => {
            if x <= pts[0][0] {
                return pts[0][1];
            }
            for w in pts.windows(2) {
                let (a, b) = (w[0], w[1]);
                if x <= b[0] {
                    let t = if b[0] > a[0] { (x - a[0]) / (b[0] - a[0]) } else { 1.0 };
                    return a[1] + (b[1] - a[1]) * t;
                }
            }
            pts[pts.len() - 1][1]
        }
    }
}

/// A parsed brush.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MybBrush {
    pub name: String,
    pub group: String,
    pub description: String,
    pub settings: BTreeMap<String, MybSetting>,
}

impl MybBrush {
    /// The setting called `key`, else a constant `default`.
    pub fn setting(&self, key: &str) -> MybSetting {
        self.settings.get(key).cloned().unwrap_or_else(|| MybSetting::constant(default_of(key)))
    }
    /// The setting's base value, else the format default.
    pub fn base(&self, key: &str) -> f32 {
        self.settings.get(key).map_or_else(|| default_of(key), |s| s.base)
    }
}

/// Format defaults for the settings this engine reads.
pub fn default_of(key: &str) -> f32 {
    match key {
        "opaque" => 1.0,
        "opaque_multiply" => 0.0,
        "opaque_linearize" => 0.9,
        "radius_logarithmic" => 2.0,
        "hardness" => 0.8,
        "anti_aliasing" => 1.0,
        "dabs_per_basic_radius" => 0.0,
        "dabs_per_actual_radius" => 2.0,
        "dabs_per_second" => 0.0,
        "radius_by_random" => 0.0,
        "speed1_slowness" => 0.04,
        "speed2_slowness" => 0.8,
        "offset_by_random" => 0.0,
        "offset_by_speed" => 0.0,
        "slow_tracking" => 0.0,
        "slow_tracking_per_dab" => 0.0,
        "tracking_noise" => 0.0,
        "color_h" | "color_s" | "color_v" => 0.0,
        "change_color_h" | "change_color_l" | "change_color_hsl_s" | "change_color_v" | "change_color_hsv_s" => 0.0,
        "smudge" => 0.0,
        "smudge_length" => 0.5,
        "smudge_radius_log" => 0.0,
        "eraser" => 0.0,
        "stroke_threshold" => 0.0,
        "stroke_duration_logarithmic" => 4.0,
        "stroke_holdtime" => 0.0,
        "custom_input" => 0.0,
        "custom_input_slowness" => 0.0,
        "elliptical_dab_ratio" => 1.0,
        "elliptical_dab_angle" => 90.0,
        "direction_filter" => 2.0,
        "lock_alpha" => 0.0,
        "colorize" => 0.0,
        "snap_to_pixel" => 0.0,
        "pressure_gain_log" => 0.0,
        _ => 0.0,
    }
}

const MAX_SOURCE: usize = 4 << 20;

/// Is `bytes` a `.myb` (JSON with a `settings` object, or the version-1 text form)?
pub fn is_myb(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(&bytes[..bytes.len().min(4096)]) else { return false };
    let t = text.trim_start();
    (t.starts_with('{') && text.contains("\"settings\"")) || t.starts_with("version 1") || t.starts_with("# mypaint brush file")
}

fn points_of(v: &serde_json::Value) -> Vec<[f32; 2]> {
    let mut pts: Vec<[f32; 2]> = v
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let q = p.as_array()?;
                    Some([q.first()?.as_f64()? as f32, q.get(1)?.as_f64()? as f32])
                })
                .filter(|p| p[0].is_finite() && p[1].is_finite())
                .collect()
        })
        .unwrap_or_default();
    pts.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
    pts
}

/// Parses a `.myb` file. `fallback_name` names a brush whose file carries no name.
pub fn parse(bytes: &[u8], fallback_name: &str) -> Result<MybBrush, IoError> {
    if bytes.len() > MAX_SOURCE {
        return Err(IoError::Unsupported("MyPaint brush file too large".into()));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| IoError::Unsupported("MyPaint brush file is not UTF-8".into()))?;
    let t = text.trim_start();
    if t.starts_with('{') {
        return parse_json(t, fallback_name);
    }
    parse_v1(text, fallback_name)
}

fn parse_json(text: &str, fallback_name: &str) -> Result<MybBrush, IoError> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| IoError::Unsupported(format!("MyPaint brush: {e}")))?;
    let settings_obj = v.get("settings").and_then(serde_json::Value::as_object).ok_or_else(|| IoError::Unsupported("MyPaint brush: no `settings`".into()))?;
    let mut settings = BTreeMap::new();
    for (k, s) in settings_obj {
        if settings.len() >= 256 {
            break;
        }
        let base = s.get("base_value").and_then(serde_json::Value::as_f64).filter(|b| b.is_finite()).unwrap_or_else(|| f64::from(default_of(k))) as f32;
        let mut inputs = BTreeMap::new();
        if let Some(obj) = s.get("inputs").and_then(serde_json::Value::as_object) {
            for (name, pts) in obj {
                let p = points_of(pts);
                if !p.is_empty() {
                    inputs.insert(name.clone(), p);
                }
            }
        }
        settings.insert(k.clone(), MybSetting { base, inputs });
    }
    let text_of = |k: &str| v.get(k).and_then(serde_json::Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    Ok(MybBrush {
        name: text_of("name").unwrap_or_else(|| fallback_name.to_string()),
        group: text_of("group").unwrap_or_default(),
        description: text_of("description").unwrap_or_default(),
        settings,
    })
}

/// Version-1 text: `setting base | input (x,y), (x,y) | input ...` per line.
fn parse_v1(text: &str, fallback_name: &str) -> Result<MybBrush, IoError> {
    let mut settings = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("version") {
            continue;
        }
        let Some((key, rest)) = line.split_once(' ') else { continue };
        let mut parts = rest.split('|');
        let base: f32 = parts.next().and_then(|b| b.trim().parse().ok()).unwrap_or_else(|| default_of(key));
        let mut inputs = BTreeMap::new();
        for part in parts {
            let part = part.trim();
            let Some((input, pts)) = part.split_once(' ') else { continue };
            let nums: Vec<f32> = pts.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e')).filter_map(|n| n.parse().ok()).collect();
            let mut curve: Vec<[f32; 2]> = nums.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
            curve.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
            if !curve.is_empty() {
                inputs.insert(input.trim().to_string(), curve);
            }
        }
        if settings.len() < 256 {
            settings.insert(key.to_string(), MybSetting { base, inputs });
        }
    }
    if settings.is_empty() {
        return Err(IoError::Unsupported("MyPaint brush: no settings found".into()));
    }
    Ok(MybBrush { name: fallback_name.to_string(), group: String::new(), description: String::new(), settings })
}

/// Serialises a brush as a version-3 `.myb`.
pub fn write(b: &MybBrush) -> String {
    let mut settings = serde_json::Map::new();
    for (k, s) in &b.settings {
        let mut inputs = serde_json::Map::new();
        for (name, pts) in &s.inputs {
            inputs.insert(name.clone(), serde_json::json!(pts));
        }
        settings.insert(k.clone(), serde_json::json!({ "base_value": s.base, "inputs": inputs }));
    }
    let v = serde_json::json!({
        "version": 3,
        "name": b.name,
        "group": b.group,
        "description": b.description,
        "comment": "Redrob Canvas MyPaint-style brush",
        "settings": settings,
    });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// The brushes shipped with the engine (no files needed): a pencil, charcoal, ink pen, airbrush,
/// marker and a soft round.
pub fn builtins() -> Vec<MybBrush> {
    let mut out = Vec::new();
    let mut mk = |name: &str, group: &str, pairs: &[(&str, f32)], pressure: &[(&str, &[[f32; 2]])]| {
        let mut settings = BTreeMap::new();
        for (k, v) in pairs {
            settings.insert((*k).to_string(), MybSetting::constant(*v));
        }
        for (k, pts) in pressure {
            let e = settings.entry((*k).to_string()).or_insert_with(|| MybSetting::constant(default_of(k)));
            e.inputs.insert("pressure".into(), pts.to_vec());
        }
        out.push(MybBrush { name: name.to_string(), group: group.to_string(), description: String::new(), settings });
    };
    mk(
        "Pencil",
        "Sketching",
        &[("radius_logarithmic", 0.7), ("hardness", 0.9), ("opaque", 0.7), ("dabs_per_actual_radius", 6.0), ("offset_by_random", 0.4)],
        &[("opaque_multiply", &[[0.0, 0.0], [1.0, 1.0]]), ("radius_logarithmic", &[[0.0, -0.3], [1.0, 0.3]])],
    );
    mk(
        "Charcoal",
        "Sketching",
        &[
            ("radius_logarithmic", 1.6),
            ("hardness", 0.3),
            ("opaque", 0.5),
            ("dabs_per_actual_radius", 4.0),
            ("offset_by_random", 1.2),
            ("radius_by_random", 0.6),
        ],
        &[("opaque_multiply", &[[0.0, 0.0], [1.0, 1.0]]), ("radius_logarithmic", &[[0.0, -0.4], [1.0, 0.5]])],
    );
    mk(
        "Ink Pen",
        "Inking",
        &[("radius_logarithmic", 1.0), ("hardness", 0.95), ("opaque", 1.0), ("dabs_per_actual_radius", 5.0), ("slow_tracking", 1.5), ("anti_aliasing", 1.0)],
        &[("radius_logarithmic", &[[0.0, -0.9], [1.0, 0.5]])],
    );
    mk(
        "Airbrush",
        "Painting",
        &[("radius_logarithmic", 3.2), ("hardness", 0.05), ("opaque", 0.15), ("dabs_per_actual_radius", 3.0), ("dabs_per_second", 80.0)],
        &[("opaque_multiply", &[[0.0, 0.0], [1.0, 1.0]])],
    );
    mk(
        "Marker",
        "Painting",
        &[
            ("radius_logarithmic", 2.2),
            ("hardness", 0.85),
            ("opaque", 0.6),
            ("dabs_per_actual_radius", 4.0),
            ("elliptical_dab_ratio", 2.5),
            ("elliptical_dab_angle", 45.0),
        ],
        &[],
    );
    mk(
        "Soft Round",
        "Painting",
        &[("radius_logarithmic", 2.8), ("hardness", 0.4), ("opaque", 0.9), ("dabs_per_actual_radius", 3.0)],
        &[("opaque_multiply", &[[0.0, 0.0], [1.0, 1.0]]), ("radius_logarithmic", &[[0.0, -0.6], [1.0, 0.4]])],
    );
    mk(
        "Eraser",
        "Erasing",
        &[("radius_logarithmic", 2.4), ("hardness", 0.5), ("opaque", 1.0), ("dabs_per_actual_radius", 3.0), ("eraser", 1.0)],
        &[("opaque_multiply", &[[0.0, 0.0], [1.0, 1.0]])],
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_and_v1_files_parse_and_round_trip() {
        let json = r##"{"version": 3, "name": "Test", "group": "g", "settings": {"opaque": {"base_value": 0.5, "inputs": {"pressure": [[0.0, -0.5], [1.0, 0.5]]}}, "radius_logarithmic": {"base_value": 2.5, "inputs": {}}}}"##;
        assert!(is_myb(json.as_bytes()));
        let b = parse(json.as_bytes(), "fallback").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(b.name, "Test");
        let o = b.setting("opaque");
        assert!((o.eval(&|k| if k == "pressure" { 1.0 } else { 0.0 }) - 1.0).abs() < 1e-6);
        assert!((o.eval(&|k| if k == "pressure" { 0.0 } else { 0.0 }) - 0.0).abs() < 1e-6);
        assert!((b.base("hardness") - 0.8).abs() < 1e-6, "format default");
        let text = write(&b);
        let back = parse(text.as_bytes(), "x").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back.settings, b.settings);
        let v1 = "version 1\nopaque 0.6 | pressure (0.0, -0.6), (1.0, 0.4)\nradius_logarithmic 1.2\n";
        assert!(is_myb(v1.as_bytes()));
        let b1 = parse(v1.as_bytes(), "Old").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(b1.name, "Old");
        assert!((b1.base("radius_logarithmic") - 1.2).abs() < 1e-6);
        assert_eq!(b1.setting("opaque").inputs["pressure"].len(), 2);
        assert_eq!(builtins().len(), 7);
    }
}
