//! Edit › Fill with Foreground / Background Color / Pattern (Opacity): the fill keys' variants
//! that honour the brush's opacity instead of filling at 100 %. Each one is `edit.fill` with the
//! contents fixed and `opacity` taken from the current brush (the options bar's Opacity), as one
//! "Fill" history step on the Channels panel target.

use serde_json::{Map, Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The command ids.
pub const IDS: [&str; 3] = ["edit.fillForegroundOpacity", "edit.fillBackgroundOpacity", "edit.fillPatternOpacity"];

const PARAMS: &str = r##"{"layer":id?,"opacity":1..100? (default: the brush's opacity),"pattern":id|name? (the pattern variant; default: the Patterns panel selection),"target":"pixels"|"mask"|"quickMask"|{"channel":i}?}"##;

/// The brush's opacity in percent, as `edit.fill` takes it.
pub fn brush_opacity_percent(s: &Session) -> f32 {
    (s.tools.brush.opacity.clamp(0.0, 1.0) * 100.0).max(1.0)
}

fn fill(s: &mut Session, p: &Value, cmd: &str, contents: &str) -> Result<Value> {
    let mut q: Map<String, Value> = match p {
        Value::Object(m) => m.clone(),
        Value::Null => Map::new(),
        _ => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "expected an object".into() }),
    };
    for k in ["color", "contents", "preserveTransparency"] {
        q.remove(k);
    }
    q.insert("contents".into(), json!(contents));
    let opacity = match q.get("opacity") {
        None | Some(Value::Null) => brush_opacity_percent(s),
        Some(v) => v
            .as_f64()
            .filter(|o| o.is_finite())
            .ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: format!("`opacity` must be a number 1..100, not {v}") })? as f32,
    };
    q.insert("opacity".into(), json!(opacity.clamp(1.0, 100.0)));
    let edit_fill = crate::commands::find("edit.fill").ok_or_else(|| EngineError::Other("Fill is unavailable".into()))?;
    (edit_fill.run)(s, &Value::Object(q))
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    let edit_fill = crate::commands::find("edit.fill").ok_or_else(|| "Fill is unavailable".to_string())?;
    (edit_fill.enabled)(s)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "edit.fillForegroundOpacity",
            label: "Fill with Foreground Color (Opacity)",
            menu: &["Edit"],
            shortcut: None,
            params: PARAMS,
            enabled,
            run: |s, p| fill(s, p, "edit.fillForegroundOpacity", "foreground"),
            journal: true,
        },
        CommandSpec {
            id: "edit.fillBackgroundOpacity",
            label: "Fill with Background Color (Opacity)",
            menu: &["Edit"],
            shortcut: None,
            params: PARAMS,
            enabled,
            run: |s, p| fill(s, p, "edit.fillBackgroundOpacity", "background"),
            journal: true,
        },
        CommandSpec {
            id: "edit.fillPatternOpacity",
            label: "Fill with Pattern (Opacity)",
            menu: &["Edit"],
            shortcut: None,
            params: PARAMS,
            enabled,
            run: |s, p| fill(s, p, "edit.fillPatternOpacity", "pattern"),
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
    }

    #[test]
    fn opacity_variants_fill_at_the_brush_opacity() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
        s.tools.foreground = [1.0, 0.0, 0.0, 1.0];
        s.tools.background = [0.0, 0.0, 1.0, 1.0];
        s.tools.brush.opacity = 0.5;
        s.execute("edit.fillForegroundOpacity", json!({})).unwrap();
        let p = px(&s, 2, 2);
        assert!((p[0] - 1.0).abs() < 0.01 && (p[1] - 0.5).abs() < 0.02 && (p[2] - 0.5).abs() < 0.02, "half red over white: {p:?}");
        s.execute("edit.fillBackgroundOpacity", json!({"opacity": 100})).unwrap();
        assert_eq!(px(&s, 2, 2), vec![0.0, 0.0, 1.0, 1.0], "an explicit opacity wins");
        assert!(s.execute("edit.fillForegroundOpacity", json!({"opacity": "half"})).is_err());
        assert!(s.execute("edit.fillPatternOpacity", json!({"pattern": "Diagonal Lines"})).is_ok());
        for id in IDS {
            assert!(crate::commands::find(id).is_some(), "{id}");
        }
    }
}
