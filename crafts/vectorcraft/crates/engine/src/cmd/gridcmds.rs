//! Document grids (`grid.list`, `grid.add`, `grid.set`, `grid.remove`): any number of grids per
//! document, each rectangular, axonometric or modular with its own spacing, subdivisions, angles,
//! gaps, origin, colour and visibility. Grid 0 is the document's main grid (the one Preferences ›
//! Guides & Grid sets for new documents); the others are the Grids panel's. View › Show Grid draws
//! every visible grid and Snap to Grid snaps to the nearest crossing of any of them.

use redrob_vector_doc::{GridKind, GridPrefs};
use redrob_vector_geom::Point;
use serde_json::{Value, json};

use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "grid.list",
            "Grids",
            [],
            None,
            "{} → {grids: [{index, kind, spacing, subdivisions, angleX, angleZ, gapX, gapY, origin, color, visible, name}]} the document's grids, the main grid first",
            has_doc,
            |s, _| Ok(json!({ "grids": s.doc()?.doc.grids().enumerate().map(|(i, g)| grid_json(i, g)).collect::<Vec<_>>() }))
        ),
        cmd!(
            "grid.add",
            "Add Grid",
            [],
            None,
            "{kind?: \"rectangular\"|\"axonometric\"|\"modular\", spacing?: pt (a gridline every), subdivisions?: n, angleX?, angleZ?: deg (axonometric, 1..89), gapX?, gapY?: pt (modular), origin?: [x, y] (where the lines start), color?: \"#rrggbb\"|null (the preferences' colour), visible?: bool, name?: string} add a grid to the document (defaults: a copy of the main grid) → {index}",
            has_doc,
            add
        ),
        cmd!(
            "grid.set",
            "Grid Options",
            [],
            None,
            "{index: n (0: the main grid), kind?: \"rectangular\"|\"axonometric\"|\"modular\", spacing?: pt (a gridline every), subdivisions?: n, angleX?, angleZ?: deg (axonometric, 1..89), gapX?, gapY?: pt (modular), origin?: [x, y] (where the lines start), color?: \"#rrggbb\"|null (the preferences' colour), visible?: bool, name?: string} change a grid → {index}",
            has_doc,
            set
        ),
        cmd!("grid.remove", "Remove Grid", [], None, "{index: n (1..; the main grid stays)} remove a grid → {index}", has_doc, remove),
    ]
}

fn grid_json(i: usize, g: &GridPrefs) -> Value {
    json!({
        "index": i,
        "kind": kind_id(g.kind),
        "spacing": g.spacing,
        "subdivisions": g.subdivisions,
        "angleX": g.angle_x,
        "angleZ": g.angle_z,
        "gapX": g.gap_x,
        "gapY": g.gap_y,
        "origin": [g.origin.x, g.origin.y],
        "color": g.color,
        "visible": g.visible,
        "name": g.name,
    })
}

fn kind_id(k: GridKind) -> &'static str {
    match k {
        GridKind::Rectangular => "rectangular",
        GridKind::Axonometric => "axonometric",
        GridKind::Modular => "modular",
    }
}

/// Apply the grid fields of `p` to `g`.
fn apply(cmd: &str, g: &mut GridPrefs, p: &Value) -> Result<()> {
    if let Some(k) = str_param(p, "kind") {
        g.kind = match k {
            "rectangular" => GridKind::Rectangular,
            "axonometric" => GridKind::Axonometric,
            "modular" => GridKind::Modular,
            other => return Err(bad(cmd, format!("kind must be rectangular, axonometric or modular, not `{other}`"))),
        };
    }
    g.spacing = f64_or(p, "spacing", g.spacing).clamp(0.01, 16383.0);
    if let Some(n) = p.get("subdivisions").and_then(Value::as_u64) {
        g.subdivisions = n.clamp(1, 1000) as u32;
    }
    g.angle_x = f64_or(p, "angleX", g.angle_x).clamp(1.0, 89.0);
    g.angle_z = f64_or(p, "angleZ", g.angle_z).clamp(1.0, 89.0);
    g.gap_x = f64_or(p, "gapX", g.gap_x).clamp(0.0, 16383.0);
    g.gap_y = f64_or(p, "gapY", g.gap_y).clamp(0.0, 16383.0);
    if let Some(o) = p.get("origin").and_then(Value::as_array) {
        let (Some(x), Some(y)) = (o.first().and_then(Value::as_f64), o.get(1).and_then(Value::as_f64)) else {
            return Err(bad(cmd, "origin must be [x, y]"));
        };
        g.origin = Point::new(x, y);
    }
    match p.get("color") {
        Some(Value::Null) => g.color = None,
        Some(Value::String(c)) => {
            if redrob_vector_doc::color::Color::from_hex(c).is_none() {
                return Err(bad(cmd, format!("color must be #rrggbb, not `{c}`")));
            }
            g.color = Some(c.clone());
        }
        Some(_) => return Err(bad(cmd, "color must be #rrggbb or null")),
        None => {}
    }
    g.visible = bool_or(p, "visible", g.visible);
    if let Some(n) = str_param(p, "name") {
        g.name = n.trim().to_string();
    }
    Ok(())
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "grid.add";
    let mut g = s.doc()?.doc.grid.clone();
    g.name = String::new();
    apply(C, &mut g, p)?;
    let index = s.edit("Add Grid", |d, _| {
        d.extra_grids.push(g);
        Ok(d.extra_grids.len())
    })?;
    Ok(json!({ "index": index }))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "grid.set";
    let index = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad(C, "missing `index`"))? as usize;
    let mut g = s.doc()?.doc.grids().nth(index).cloned().ok_or_else(|| bad(C, format!("no grid {index}")))?;
    apply(C, &mut g, p)?;
    s.edit("Grid Options", |d, _| {
        if let Some(slot) = d.grid_mut(index) {
            *slot = g;
        }
        Ok(())
    })?;
    Ok(json!({ "index": index }))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "grid.remove";
    let index = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad(C, "missing `index`"))? as usize;
    if index == 0 {
        return Err(bad(C, "the main grid stays; set its fields with grid.set"));
    }
    if index > s.doc()?.doc.extra_grids.len() {
        return Err(bad(C, format!("no grid {index}")));
    }
    s.edit("Remove Grid", |d, _| {
        d.extra_grids.remove(index - 1);
        Ok(())
    })?;
    Ok(json!({ "index": index }))
}
