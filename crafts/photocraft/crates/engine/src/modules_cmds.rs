//! Edit › Modules… and Edit › Units… (Photo mode, v0.11 parity).
//!
//! Modules: the loadable modules of this build are the installed WebAssembly plug-ins. Each one
//! can be disabled without uninstalling it: a disabled module keeps its file but leaves the
//! Filter › Plug-ins menu and refuses `plugin.run` until it is enabled again. The disabled set
//! persists in Preferences › Plug-ins.
//!
//! Units: beside the built-in ruler units, custom measurement units (identifier, factor in units
//! per inch, digits, symbol, abbreviation, singular and plural names) are defined here, persist
//! in Preferences › Units and Rulers, and are offered wherever a length is read out through
//! [`crate::prefs::UnitsAndRulers::format_custom`].

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::prefs::CustomUnit;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// Is plug-in `id` enabled (not in the disabled set)?
pub fn module_enabled(s: &Session, id: &str) -> bool {
    !s.prefs().plug_ins.disabled_modules.iter().any(|d| d == id)
}

fn module_list(s: &mut Session, _p: &Value) -> Result<Value> {
    let disabled = s.prefs().plug_ins.disabled_modules.clone();
    let mut items: Vec<Value> = redrob_photo_plugins::registry::list()
        .iter()
        .map(|p| {
            let m = p.manifest();
            json!({
                "id": m.id,
                "name": m.name,
                "version": m.version,
                "kind": "wasm-filter",
                "enabled": !disabled.iter().any(|d| *d == m.id),
                "loaded": true,
            })
        })
        .collect();
    // Disabled modules whose file is gone still show, so the setting can be cleared.
    for d in &disabled {
        if !items.iter().any(|i| i.get("id").and_then(Value::as_str) == Some(d)) {
            items.push(json!({ "id": d, "name": d, "version": "", "kind": "wasm-filter", "enabled": false, "loaded": false }));
        }
    }
    Ok(json!({ "modules": items }))
}

fn module_set_enabled(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "module.setEnabled";
    let id = p.get("id").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).ok_or_else(|| bad(CMD, "pass `id`"))?.to_string();
    let enabled = p.get("enabled").and_then(Value::as_bool).ok_or_else(|| bad(CMD, "pass `enabled` (bool)"))?;
    s.edit_prefs(|prefs| {
        let d = &mut prefs.plug_ins.disabled_modules;
        d.retain(|x| *x != id);
        if !enabled {
            d.push(id.clone());
            d.sort();
        }
    });
    Ok(json!({ "id": id, "enabled": enabled }))
}

fn unit_param(p: &Value, cmd: &str) -> Result<CustomUnit> {
    let identifier = p.get("identifier").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).ok_or_else(|| bad(cmd, "pass `identifier`"))?;
    if identifier.len() > 32 || !identifier.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return Err(bad(cmd, "`identifier` is letters, digits, - and _ (at most 32)"));
    }
    let factor = p.get("factor").and_then(Value::as_f64).ok_or_else(|| bad(cmd, "pass `factor` (units per inch)"))?;
    if !factor.is_finite() || factor <= 0.0 || factor > 1e9 {
        return Err(bad(cmd, "`factor` must be a positive number of units per inch"));
    }
    let digits = p.get("digits").and_then(Value::as_u64).unwrap_or(2).min(8) as u8;
    let text = |k: &str, d: &str| p.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).unwrap_or(d).chars().take(32).collect::<String>();
    Ok(CustomUnit {
        identifier: identifier.to_string(),
        factor,
        digits,
        symbol: text("symbol", identifier),
        abbreviation: text("abbreviation", identifier),
        singular: text("singular", identifier),
        plural: text("plural", identifier),
    })
}

fn unit_list(s: &mut Session, _p: &Value) -> Result<Value> {
    let builtin: Vec<Value> = crate::prefs::Unit::ALL
        .iter()
        .map(|u| json!({ "identifier": u.suffix(), "factor": u.to_px(1.0, 1.0, 1.0, 72.0).recip(), "digits": u.decimals(), "symbol": u.suffix(), "builtin": true }))
        .collect();
    let custom: Vec<Value> = s
        .prefs()
        .units_and_rulers
        .custom_units
        .iter()
        .map(|u| {
            json!({
                "identifier": u.identifier, "factor": u.factor, "digits": u.digits, "symbol": u.symbol, "abbreviation": u.abbreviation,
                "singular": u.singular, "plural": u.plural, "builtin": false
            })
        })
        .collect();
    Ok(json!({ "builtin": builtin, "custom": custom }))
}

fn unit_set(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "units.set";
    let unit = unit_param(p, CMD)?;
    if crate::prefs::Unit::ALL.iter().any(|u| u.suffix().eq_ignore_ascii_case(&unit.identifier)) {
        return Err(bad(CMD, format!("`{}` is a built-in unit", unit.identifier)));
    }
    if s.prefs().units_and_rulers.custom_units.len() >= 64 && !s.prefs().units_and_rulers.custom_units.iter().any(|u| u.identifier == unit.identifier) {
        return Err(bad(CMD, "at most 64 custom units"));
    }
    let id = unit.identifier.clone();
    let replaced = s.edit_prefs(|prefs| {
        let list = &mut prefs.units_and_rulers.custom_units;
        match list.iter_mut().find(|u| u.identifier == unit.identifier) {
            Some(u) => {
                *u = unit.clone();
                true
            }
            None => {
                list.push(unit.clone());
                false
            }
        }
    });
    Ok(json!({ "identifier": id, "replaced": replaced }))
}

fn unit_delete(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "units.delete";
    let id = p.get("identifier").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).ok_or_else(|| bad(CMD, "pass `identifier`"))?.to_string();
    let removed = s.edit_prefs(|prefs| {
        let list = &mut prefs.units_and_rulers.custom_units;
        let before = list.len();
        list.retain(|u| u.identifier != id);
        before != list.len()
    });
    if !removed {
        return Err(bad(CMD, format!("no custom unit `{id}`")));
    }
    Ok(json!({ "deleted": id }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "module.list",
            label: "List Modules",
            menu: &[],
            shortcut: None,
            params: r##"{} → {modules: [{id, name, version, kind, enabled, loaded}]}"##,
            enabled: always,
            run: module_list,
            journal: false,
        },
        CommandSpec {
            id: "module.setEnabled",
            label: "Enable or Disable Module",
            menu: &[],
            shortcut: None,
            params: r##"{"id":text,"enabled":bool} → {id, enabled}"##,
            enabled: always,
            run: module_set_enabled,
            journal: false,
        },
        CommandSpec {
            id: "units.list",
            label: "List Units",
            menu: &[],
            shortcut: None,
            params: r##"{} → {builtin: [...], custom: [{identifier, factor, digits, symbol, abbreviation, singular, plural}]}"##,
            enabled: always,
            run: unit_list,
            journal: false,
        },
        CommandSpec {
            id: "units.set",
            label: "Define Unit",
            menu: &[],
            shortcut: None,
            params: r##"{"identifier":text,"factor":number (units per inch),"digits":0-8=2,"symbol":text?,"abbreviation":text?,"singular":text?,"plural":text?} → {identifier, replaced}"##,
            enabled: always,
            run: unit_set,
            journal: false,
        },
        CommandSpec {
            id: "units.delete",
            label: "Delete Unit",
            menu: &[],
            shortcut: None,
            params: r##"{"identifier":text} → {deleted}"##,
            enabled: always,
            run: unit_delete,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_units_persist_and_format() {
        let mut s = Session::new();
        s.execute("units.set", json!({"identifier": "furlong", "factor": 0.000126262626, "digits": 4, "symbol": "fur"})).unwrap_or_else(|e| panic!("{e}"));
        let l = s.execute("units.list", json!({})).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(l["custom"].as_array().map(Vec::len), Some(1));
        let u = &s.prefs().units_and_rulers.custom_units[0];
        // 7920 inches (one furlong) at 72 dpi.
        let text = s.prefs().units_and_rulers.format_custom(u, 7920.0 * 72.0, 72.0);
        assert_eq!(text, "1 fur");
        assert!(s.execute("units.set", json!({"identifier": "px", "factor": 72.0})).is_err(), "built-in units are not redefined");
        s.execute("units.delete", json!({"identifier": "furlong"})).unwrap_or_else(|e| panic!("{e}"));
        assert!(s.prefs().units_and_rulers.custom_units.is_empty());
    }

    #[test]
    fn modules_can_be_disabled_without_being_installed() {
        let mut s = Session::new();
        s.execute("module.setEnabled", json!({"id": "demo.filter", "enabled": false})).unwrap_or_else(|e| panic!("{e}"));
        assert!(!module_enabled(&s, "demo.filter"));
        let l = s.execute("module.list", json!({})).unwrap_or_else(|e| panic!("{e}"));
        assert!(l["modules"].as_array().is_some_and(|m| m.iter().any(|x| x["id"] == "demo.filter" && x["enabled"] == false)));
        s.execute("module.setEnabled", json!({"id": "demo.filter", "enabled": true})).unwrap_or_else(|e| panic!("{e}"));
        assert!(module_enabled(&s, "demo.filter"));
    }
}
