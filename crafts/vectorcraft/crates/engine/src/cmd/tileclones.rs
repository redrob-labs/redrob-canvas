//! Object › Clone › Create Tiled Clones…: rows × columns of clones (or copies) of the selected
//! object laid out by one of the 17 wallpaper symmetry groups, with per-row and per-column
//! shift, scale, rotation, opacity and colour increments, randomisation, and tracing of the
//! drawing under each tile (its colour, opacity or size taken from the pixels there).

use redrob_vector_color::{Color, Paint};
use redrob_vector_doc::recolor::{ColorVisitor, Reach, recolor_node};
use redrob_vector_doc::{CloneRef, Document, Node};
use redrob_vector_geom::{Affine, Point, Rect, Vec2};
use serde_json::{Value, json};

use super::edit::selected_roots;
use super::*;

/// The wallpaper groups, as the dialog lists them.
pub const SYMMETRIES: [&str; 17] = ["P1", "P2", "PM", "PG", "CM", "PMM", "PMG", "PGG", "CMM", "P4", "P4M", "P4G", "P3", "P31M", "P3M1", "P6", "P6M"];

/// The most clones one command makes.
const MAX_CLONES: usize = 10_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "object.tileClones",
        "Create Tiled Clones…",
        ["Object", "Clone"],
        None,
        "{rows: 2, cols: 2, symmetry: P1|P2|PM|PG|CM|PMM|PMG|PGG|CMM|P4|P4M|P4G|P3|P31M|P3M1|P6|P6M (default P1), shiftRowX?: 0, shiftRowY?: 0, shiftColX?: 0, shiftColY?: 0 (% of the tile's size added per row / per column), shiftRandom?: 0 (%), scaleRow?: 0, scaleCol?: 0 (% per row / column), scaleRandom?: 0, rotateRow?: 0, rotateCol?: 0 (degrees per row / column), rotateRandom?: 0, opacityRow?: 0, opacityCol?: 0 (% less opacity per row / column), opacityRandom?: 0, hueRow?: 0, hueCol?: 0 (degrees per row / column), saturationRow?: 0, saturationCol?: 0, lightnessRow?: 0, lightnessCol?: 0 (%), colorRandom?: 0, trace?: none|color|opacity|size (take the drawing's pixels under each tile's centre as its colour, its opacity or its scale), clones?: true (clones of the original; false: plain copies), seed?: 1 (the randomisation), ids?} tile the selected object → {ids, rows, cols}",
        has_selection,
        tile
    )]
}

/// One tile's placement: the lattice offset in tile units and the point-group operation.
#[derive(Clone, Copy)]
struct Op {
    /// Translation in units of the tile's width and height.
    dx: f64,
    dy: f64,
    /// Rotation (degrees) about the tile's centre.
    rot: f64,
    flip_x: bool,
    flip_y: bool,
}

/// The lattice operation of tile (row `i`, column `j`) in `group`: how the 17 groups lay their
/// fundamental domain out when one object is the domain.
fn op(group: &str, i: i64, j: i64) -> Op {
    let (fi, fj) = (i as f64, j as f64);
    let odd = |v: i64| v.rem_euclid(2) == 1;
    let hex = |i: f64, j: f64| (j + i * 0.5, i * 0.866_025_4);
    let mut o = Op { dx: fj, dy: fi, rot: 0.0, flip_x: false, flip_y: false };
    match group {
        "P1" => {}
        "P2" => o.rot = if odd(j) { 180.0 } else { 0.0 },
        "PM" => o.flip_x = odd(j),
        "PG" => o.flip_y = odd(j),
        "CM" => {
            o.flip_x = odd(j);
            if odd(i) {
                o.dx += 0.5;
            }
        }
        "PMM" => {
            o.flip_x = odd(j);
            o.flip_y = odd(i);
        }
        "PMG" => {
            o.rot = if odd(j) { 180.0 } else { 0.0 };
            o.flip_y = odd(i);
        }
        "PGG" => {
            o.rot = if odd(j) != odd(i) { 180.0 } else { 0.0 };
            o.flip_x = odd(i);
        }
        "CMM" => {
            o.flip_x = odd(j);
            o.flip_y = odd(i);
            if odd(i) {
                o.dx += 0.5;
            }
        }
        "P4" => o.rot = 90.0 * ((2 * i.rem_euclid(2) + j.rem_euclid(2)) as f64),
        "P4M" => {
            o.rot = 90.0 * ((2 * i.rem_euclid(2) + j.rem_euclid(2)) as f64);
            o.flip_x = (i + j).rem_euclid(2) == 1;
        }
        "P4G" => {
            o.rot = 90.0 * ((2 * i.rem_euclid(2) + j.rem_euclid(2)) as f64);
            o.flip_y = odd(i);
        }
        "P3" => {
            (o.dx, o.dy) = hex(fi, fj);
            o.rot = 120.0 * ((i + j).rem_euclid(3) as f64);
        }
        "P31M" => {
            (o.dx, o.dy) = hex(fi, fj);
            o.rot = 120.0 * ((i + j).rem_euclid(3) as f64);
            o.flip_x = odd(i);
        }
        "P3M1" => {
            (o.dx, o.dy) = hex(fi, fj);
            o.rot = 120.0 * ((i + j).rem_euclid(3) as f64);
            o.flip_x = odd(j);
        }
        "P6" => {
            (o.dx, o.dy) = hex(fi, fj);
            o.rot = 60.0 * ((i + j).rem_euclid(6) as f64);
        }
        "P6M" => {
            (o.dx, o.dy) = hex(fi, fj);
            o.rot = 60.0 * ((i + j).rem_euclid(6) as f64);
            o.flip_x = odd(i + j);
        }
        _ => {}
    }
    o
}

/// A small deterministic pseudo-random source (SplitMix64).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }
    /// -1 .. 1.
    fn signed(&mut self) -> f64 {
        self.next() * 2.0 - 1.0
    }
}

/// Shifts colours in HSB.
struct Shift {
    hue: f64,
    sat: f64,
    light: f64,
    /// A replacement colour (trace: color).
    replace: Option<Color>,
}

impl ColorVisitor for Shift {
    fn paint(&mut self, p: &mut Paint) -> bool {
        let (hue, sat, light, replace) = (self.hue, self.sat, self.light, self.replace);
        p.map_links(&mut |c, link, _| {
            *c = match replace {
                Some(r) => r,
                None => shifted(*c, hue, sat, light),
            };
            *link = None;
            true
        })
    }
    fn color(&mut self, c: Color) -> Color {
        match self.replace {
            Some(r) => r,
            None => shifted(c, self.hue, self.sat, self.light),
        }
    }
    fn image(&mut self, _: &str) -> Option<String> {
        None
    }
}

fn shifted(c: Color, hue: f64, sat: f64, light: f64) -> Color {
    let [h, s, v] = c.to_hsb();
    let h = (f64::from(h) + hue).rem_euclid(360.0) as f32;
    let s = (f64::from(s) + sat / 100.0).clamp(0.0, 1.0) as f32;
    let v = (f64::from(v) + light / 100.0).clamp(0.0, 1.0) as f32;
    Color::from_hsb(h, s, v)
}

/// The pixel of the drawing under `at` (straight RGBA), or none off the art.
fn sample(doc: &Document, at: Point, except: NodeId) -> Option<[u8; 4]> {
    let mut r = redrob_vector_render::Renderer::new();
    let opts = redrob_vector_render::RenderOptions { background: None, hidden: vec![except], skip_templates: true, ..Default::default() };
    let img = r.render_region_with(doc, Rect::new(at.x - 0.5, at.y - 0.5, at.x + 0.5, at.y + 0.5), 1.0, &opts);
    let px = img.to_straight();
    px.get(..4).map(|p| [p[0], p[1], p[2], p[3]])
}

fn tile(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.tileClones";
    let rows = p.get("rows").and_then(Value::as_u64).unwrap_or(2).clamp(1, 1000) as i64;
    let cols = p.get("cols").and_then(Value::as_u64).unwrap_or(2).clamp(1, 1000) as i64;
    if (rows * cols) as usize > MAX_CLONES {
        return Err(bad(C, format!("at most {MAX_CLONES} tiles")));
    }
    let symmetry = str_param(p, "symmetry").unwrap_or("P1").to_ascii_uppercase();
    if !SYMMETRIES.contains(&symmetry.as_str()) {
        return Err(bad(C, format!("symmetry must be one of {}", SYMMETRIES.join(", "))));
    }
    let n = |k: &str| f64_or(p, k, 0.0);
    let (shift_rx, shift_ry, shift_cx, shift_cy, shift_rand) = (n("shiftRowX"), n("shiftRowY"), n("shiftColX"), n("shiftColY"), n("shiftRandom"));
    let (scale_r, scale_c, scale_rand) = (n("scaleRow"), n("scaleCol"), n("scaleRandom"));
    let (rot_r, rot_c, rot_rand) = (n("rotateRow"), n("rotateCol"), n("rotateRandom"));
    let (op_r, op_c, op_rand) = (n("opacityRow"), n("opacityCol"), n("opacityRandom"));
    let (hue_r, hue_c, sat_r, sat_c, li_r, li_c, color_rand) =
        (n("hueRow"), n("hueCol"), n("saturationRow"), n("saturationCol"), n("lightnessRow"), n("lightnessCol"), n("colorRandom"));
    let trace = str_param(p, "trace").unwrap_or("none").to_ascii_lowercase();
    if !matches!(trace.as_str(), "none" | "color" | "opacity" | "size") {
        return Err(bad(C, "trace must be none, color, opacity or size"));
    }
    let clones = bool_or(p, "clones", true);
    let mut rng = Rng(p.get("seed").and_then(Value::as_u64).unwrap_or(1).wrapping_mul(0x2545_f491_4f6c_dd1d));
    let roots = if p.get("ids").is_some() || p.get("id").is_some() { targets(s, p)? } else { selected_roots(s)? };
    let root = *roots.first().ok_or_else(|| bad(C, "select the object to tile"))?;
    let (original, bounds) = {
        let d = &s.doc()?.doc;
        let n = d.node(root).ok_or_else(|| bad(C, "no such object"))?;
        if n.is_layer() {
            return Err(bad(C, "select an object, not a layer"));
        }
        (n.clone(), n.visual_bounds().ok_or_else(|| bad(C, "the object has no size"))?)
    };
    let (w, h) = (bounds.width().max(1e-6), bounds.height().max(1e-6));
    let center = bounds.center();
    let samples_doc = (trace != "none").then(|| s.doc().map(|st| (*st.doc).clone())).transpose()?;
    let ids = s.edit("Create Tiled Clones", |d, sel| {
        let Some((parent, index, _)) = d.position(root) else { return Err(EngineError::Other("the object is gone".into())) };
        let mut made = vec![];
        let mut at = index + 1;
        for i in 0..rows {
            for j in 0..cols {
                if i == 0 && j == 0 {
                    continue;
                }
                let o = op(&symmetry, i, j);
                let (fi, fj) = (i as f64, j as f64);
                // Shift: the lattice, plus the per-row / per-column increments and the randomness.
                let dx = (o.dx + (shift_rx * fi + shift_cx * fj) / 100.0 + shift_rand / 100.0 * rng.signed()) * w;
                let dy = (o.dy + (shift_ry * fi + shift_cy * fj) / 100.0 + shift_rand / 100.0 * rng.signed()) * h;
                let mut scale = 1.0 + (scale_r * fi + scale_c * fj) / 100.0 + scale_rand / 100.0 * rng.signed();
                let rot = o.rot + rot_r * fi + rot_c * fj + rot_rand * rng.signed();
                let mut opacity = 1.0 - (op_r * fi + op_c * fj) / 100.0 - op_rand / 100.0 * rng.signed();
                let mut shift = Shift {
                    hue: hue_r * fi + hue_c * fj + color_rand * 3.6 * rng.signed(),
                    sat: sat_r * fi + sat_c * fj + color_rand * rng.signed(),
                    light: li_r * fi + li_c * fj + color_rand * rng.signed(),
                    replace: None,
                };
                let tile_center = center + Vec2::new(dx, dy);
                if let Some(doc) = &samples_doc
                    && let Some([r, g, b, a]) = sample(doc, tile_center, root)
                {
                    match trace.as_str() {
                        "color" if a > 0 => shift.replace = Some(Color::rgb8(r, g, b)),
                        "opacity" => opacity *= f64::from(a) / 255.0,
                        "size" => {
                            let lum = (0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b)) / 255.0 * f64::from(a) / 255.0;
                            scale *= lum.max(0.05);
                        }
                        _ => {}
                    }
                }
                let scale = scale.clamp(0.01, 100.0);
                let xf = Affine::translate(tile_center.to_vec2())
                    * Affine::rotate(rot.to_radians())
                    * Affine::scale_non_uniform(if o.flip_x { -scale } else { scale }, if o.flip_y { -scale } else { scale })
                    * Affine::translate(-center.to_vec2());
                let mut c: Node = d.reid(&original);
                c.name = None;
                c.transform(xf, true);
                c.opacity = (c.opacity * opacity.clamp(0.0, 1.0) as f32).clamp(0.0, 1.0);
                let recolored = shift.hue != 0.0 || shift.sat != 0.0 || shift.light != 0.0 || shift.replace.is_some();
                if recolored {
                    recolor_node(&mut c, Reach { fill: true, stroke: true, images: false }, &mut shift);
                }
                if clones {
                    c.extras_mut().clone_of = Some(Box::new(CloneRef { original: root, xf, own_paint: recolored }));
                }
                let id = d.insert(parent, at, c)?;
                at += 1;
                made.push(id);
            }
        }
        sel.set(made.iter().copied());
        Ok(made)
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "rows": rows, "cols": cols }))
}
