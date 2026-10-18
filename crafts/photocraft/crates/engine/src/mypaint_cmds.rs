//! MyPaint brushes: `.myb` files read into brush presets (group "MyPaint", tagged "MyPaint"),
//! listed by the MyPaint Brushes dialog.
//!
//! The `.myb` format is public: version 3 is a JSON object `{"version": 3, "group": "...",
//! "settings": {"<setting>": {"base_value": f, "inputs": {"pressure": [[x, y], …], …}}, …}}`;
//! versions 1 and 2 are text, one `<setting> <base value> | <input> (x y), (x y) …` per line with
//! `#`-comments and `version <n>` / `group <name>` headers. Only the settings a round brush has
//! here are read: `radius_logarithmic` (radius = e^value px), `hardness`, `opaque` (and
//! `opaque_multiply` when its base is above 0), `dabs_per_basic_radius` / `dabs_per_actual_radius`
//! (spacing), `color_h`/`color_s`/`color_v`, `elliptical_dab_ratio` / `elliptical_dab_angle`,
//! `eraser` (above 0.5 marks the preset an eraser), `anti_aliasing`; a pressure input on the radius
//! or the opacity turns the matching pressure switch on.
//!
//! * `brush.mypaint.import {"name","text","group"?,"tags"?}`: one brush from its `.myb` text.
//! * `brush.mypaint.list {}`: the imported MyPaint brushes with their tags.

use redrob_photo_paint::{BrushPreset, BrushSettings, PresetCaptures};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The preset group MyPaint brushes land in.
pub const GROUP: &str = "MyPaint";

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// One parsed setting: its base value and whether a pressure input moves it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Setting {
    pub base: f32,
    pub pressure: bool,
}

/// A parsed `.myb`: its settings by name, plus the file's own group.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MybFile {
    pub group: String,
    pub settings: std::collections::BTreeMap<String, Setting>,
    pub comment: String,
}

impl MybFile {
    pub fn base(&self, name: &str, default: f32) -> f32 {
        self.settings.get(name).map_or(default, |s| s.base)
    }
    pub fn pressure(&self, name: &str) -> bool {
        self.settings.get(name).is_some_and(|s| s.pressure)
    }
}

/// Parses a `.myb` of version 1, 2 or 3.
pub fn parse_myb(text: &str) -> std::result::Result<MybFile, String> {
    let trimmed = text.trim_start_matches('\u{feff}').trim();
    if trimmed.starts_with('{') {
        return parse_v3(trimmed);
    }
    parse_text(trimmed)
}

fn parse_v3(text: &str) -> std::result::Result<MybFile, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("not a MyPaint brush: {e}"))?;
    let o = v.as_object().ok_or("not a MyPaint brush: expected an object")?;
    let mut f = MybFile { group: o.get("group").and_then(Value::as_str).unwrap_or("").to_string(), ..Default::default() };
    f.comment = o.get("comment").and_then(Value::as_str).unwrap_or("").to_string();
    let settings = o.get("settings").and_then(Value::as_object).ok_or("not a MyPaint brush: no settings")?;
    for (name, s) in settings {
        let base = s.get("base_value").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let pressure = s.get("inputs").and_then(Value::as_object).and_then(|i| i.get("pressure")).and_then(Value::as_array).is_some_and(|pts| {
            // A pressure curve that actually moves the value: two points with different y.
            let ys: Vec<f64> = pts.iter().filter_map(|p| p.as_array()?.get(1)?.as_f64()).collect();
            ys.iter().any(|y| (*y - ys[0]).abs() > 1e-6)
        });
        f.settings.insert(name.clone(), Setting { base, pressure });
    }
    Ok(f)
}

fn parse_text(text: &str) -> std::result::Result<MybFile, String> {
    let mut f = MybFile::default();
    let mut any = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (head, inputs) = line.split_once('|').map_or((line, ""), |(h, i)| (h.trim(), i));
        let mut parts = head.split_whitespace();
        let Some(name) = parts.next() else { continue };
        let value = parts.next().unwrap_or("");
        match name {
            "version" => continue,
            "group" => {
                f.group = value.to_string();
                continue;
            }
            "comment" | "notes" => continue,
            _ => {}
        }
        let Ok(base) = value.parse::<f32>() else { continue };
        let pressure = inputs.split('|').any(|i| {
            let i = i.trim();
            i.starts_with("pressure") && {
                let ys: Vec<f32> = i.split('(').skip(1).filter_map(|p| p.split_whitespace().nth(1)?.trim_end_matches([')', ',']).parse().ok()).collect();
                ys.iter().any(|y| (*y - ys[0]).abs() > 1e-6)
            }
        });
        f.settings.insert(name.to_string(), Setting { base, pressure });
        any = true;
    }
    if !any {
        return Err("not a MyPaint brush: no settings".into());
    }
    Ok(f)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = (h.rem_euclid(1.0)) * 6.0;
    let i = h.floor();
    let f = h - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// The brush settings a `.myb` stands for here.
pub fn to_brush(f: &MybFile) -> BrushSettings {
    let mut b = BrushSettings::default();
    let radius = f.base("radius_logarithmic", 2.0).exp();
    b.size = (radius * 2.0).clamp(1.0, 5000.0);
    b.hardness = f.base("hardness", 0.8).clamp(0.0, 1.0);
    let mut opaque = f.base("opaque", 1.0).clamp(0.0, 1.0);
    let mul = f.base("opaque_multiply", 0.0);
    if mul > 0.0 {
        opaque = (opaque * mul).clamp(0.0, 1.0);
    }
    b.opacity = opaque.max(0.01);
    b.flow = 1.0;
    // dabs per radius d: one dab every 1/d radii, i.e. a fraction 1/(2d) of the diameter.
    let dabs = f.base("dabs_per_actual_radius", 0.0).max(f.base("dabs_per_basic_radius", 0.0));
    if dabs > 0.0 {
        b.spacing = (1.0 / (2.0 * dabs)).clamp(0.01, 10.0);
        b.spacing_enabled = true;
    }
    let [r, g, bl] = hsv_to_rgb(f.base("color_h", 0.0), f.base("color_s", 0.0), f.base("color_v", 0.0));
    b.color = [r, g, bl, 1.0];
    let ratio = f.base("elliptical_dab_ratio", 1.0).max(1.0);
    b.roundness = (1.0 / ratio).clamp(0.05, 1.0);
    b.angle = f.base("elliptical_dab_angle", 90.0) - 90.0;
    b.erase = f.base("eraser", 0.0) > 0.5;
    b.aliased = f.base("anti_aliasing", 1.0) < 0.5;
    b.pressure_size = f.pressure("radius_logarithmic");
    b.pressure_opacity = f.pressure("opaque") || f.pressure("opaque_multiply");
    b
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.mypaint.import";
    let name = p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(cmd, "missing `name`"))?;
    let name = name.trim_end_matches(".myb").to_string();
    let text = p.get("text").and_then(Value::as_str).ok_or_else(|| bad(cmd, "missing `text` (the .myb file's contents)"))?;
    let f = parse_myb(text).map_err(|e| bad(cmd, e))?;
    let mut brush = to_brush(&f);
    // The preset paints with the MyPaint engine (its full dynamics), the round-brush mapping
    // above being what the tip preview and the Pixel-engine fallback show.
    if let Ok(mut settings) = redrob_photo_paint::engines::MyPaintSettings::from_myb(text) {
        if settings.name.is_empty() {
            settings.name = name.clone();
        }
        brush.engine = redrob_photo_paint::engines::EngineSettings::MyPaint(Box::new(settings));
    }
    let group = p.get("group").and_then(Value::as_str).map(str::trim).filter(|g| !g.is_empty()).unwrap_or(GROUP).to_string();
    let mut folder = Vec::new();
    if !f.group.is_empty() {
        folder.push(f.group.clone());
    }
    let preset =
        BrushPreset { name: name.clone(), brush, builtin: false, group, folder, captures: PresetCaptures { size: true, tool_settings: true, color: false } };
    match s.tools.presets.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&name)) {
        Some(x) => *x = preset,
        None => s.tools.presets.push(preset),
    }
    let mut tags: Vec<String> = vec![GROUP.into()];
    if !f.group.is_empty() {
        tags.push(f.group.clone());
    }
    if let Some(extra) = p.get("tags").and_then(Value::as_array) {
        tags.extend(extra.iter().filter_map(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string));
    }
    let key = s.tools.preset_tags.keys().find(|n| n.eq_ignore_ascii_case(&name)).cloned().unwrap_or_else(|| name.clone());
    let list = s.tools.preset_tags.entry(key).or_default();
    for t in tags {
        if !list.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
            list.push(t);
        }
    }
    list.sort_by_key(|t| t.to_lowercase());
    s.brush_presets_changed();
    Ok(
        json!({"name": name, "group": f.group, "size": s.tools.presets.iter().find(|x| x.name == name).map(|x| x.brush.size), "tags": crate::brush_preset_cmds::tags_of(s, &name)}),
    )
}

/// The imported MyPaint brushes: name, folder (the file's group), size and tags.
pub fn list_brushes(s: &Session) -> Vec<Value> {
    s.tools
        .presets
        .iter()
        .filter(|p| p.group == GROUP)
        .map(|p| json!({"name": p.name, "folder": p.folder, "size": p.brush.size, "eraser": p.brush.erase, "tags": crate::brush_preset_cmds::tags_of(s, &p.name)}))
        .collect()
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({"brushes": list_brushes(s)}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "brush.mypaint.import",
            label: "Import MyPaint Brush",
            menu: &[],
            shortcut: None,
            params: r##"{"name":string (the file name),"text":string (the .myb contents, v1-v3),"group":string="MyPaint","tags":[string]?} → {name, group, size, tags}"##,
            enabled: crate::commands::always,
            run: import,
            journal: true,
        },
        CommandSpec {
            id: "brush.mypaint.list",
            label: "List MyPaint Brushes",
            menu: &[],
            shortcut: None,
            params: "{} → {brushes: [{name, folder, size, eraser, tags}]}",
            enabled: crate::commands::always,
            run: list,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const V3: &str = r##"{"version": 3, "group": "Classic", "comment": "a test brush", "settings": {
        "radius_logarithmic": {"base_value": 2.0, "inputs": {"pressure": [[0.0, -0.5], [1.0, 0.5]]}},
        "opaque": {"base_value": 0.8, "inputs": {}},
        "hardness": {"base_value": 0.4, "inputs": {}},
        "dabs_per_basic_radius": {"base_value": 4.0, "inputs": {}},
        "color_h": {"base_value": 0.0, "inputs": {}}, "color_s": {"base_value": 1.0, "inputs": {}}, "color_v": {"base_value": 1.0, "inputs": {}},
        "elliptical_dab_ratio": {"base_value": 2.0, "inputs": {}},
        "eraser": {"base_value": 0.0, "inputs": {}}
    }}"##;

    const V2: &str = "# mypaint brush file\nversion 2\ngroup Sketch\nradius_logarithmic 1.0\nopaque 0.5 | pressure (0.0 -0.5), (1.0 0.5)\nhardness 0.9\n";

    #[test]
    fn parses_v3_and_text_files() {
        let f = parse_myb(V3).unwrap();
        assert_eq!(f.group, "Classic");
        assert!(f.pressure("radius_logarithmic") && !f.pressure("opaque"));
        let b = to_brush(&f);
        assert!((b.size - 2.0 * 2.0f32.exp()).abs() < 0.01, "{}", b.size);
        assert!((b.hardness - 0.4).abs() < 1e-6 && (b.opacity - 0.8).abs() < 1e-6);
        assert!((b.spacing - 0.125).abs() < 1e-6);
        assert_eq!(b.color, [1.0, 0.0, 0.0, 1.0]);
        assert!((b.roundness - 0.5).abs() < 1e-6);
        assert!(b.pressure_size && !b.pressure_opacity && !b.erase);
        let f = parse_myb(V2).unwrap();
        assert_eq!(f.group, "Sketch");
        let b = to_brush(&f);
        assert!(b.pressure_opacity && !b.pressure_size);
        assert!((b.size - 2.0 * 1.0f32.exp()).abs() < 0.01);
        assert!(parse_myb("hello").is_err());
    }

    #[test]
    fn import_makes_a_tagged_preset_in_the_mypaint_group() {
        let mut s = Session::new();
        let r = s.execute("brush.mypaint.import", json!({"name": "Round Classic.myb", "text": V3, "tags": ["Ink"]})).unwrap();
        assert_eq!(r["name"], "Round Classic");
        assert_eq!(r["tags"], json!(["Classic", "Ink", "MyPaint"]));
        let p = s.tools.presets.iter().find(|p| p.name == "Round Classic").unwrap();
        assert_eq!(p.group, GROUP);
        assert_eq!(p.folder, vec!["Classic"]);
        let l = s.execute("brush.mypaint.list", json!({})).unwrap();
        assert_eq!(l["brushes"].as_array().unwrap().len(), 1);
        // Re-importing replaces, never duplicates.
        s.execute("brush.mypaint.import", json!({"name": "Round Classic", "text": V3})).unwrap();
        assert_eq!(s.tools.presets.iter().filter(|p| p.name == "Round Classic").count(), 1);
    }
}
