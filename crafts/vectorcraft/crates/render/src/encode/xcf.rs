//! Layered GIMP XCF export, written from the public XCF file format description (version 11:
//! 64-bit offsets, 8-bit gamma RGBA, uncompressed tiles). Each top-level layer of the document is
//! rendered alone and written as an XCF layer of the same name, opacity, visibility and the
//! nearest blend mode, cropped to the pixels it paints; a flat export writes one layer.

use std::sync::Arc;

use redrob_vector_color::BlendMode;
use redrob_vector_doc::{Document, LayerColor, Node, NodeKind};
use redrob_vector_geom::Rect;

use crate::{RenderOptions, Renderer};

/// Most pixels along a side.
pub const MAX_SIDE: u32 = 262_144;
/// XCF tiles are 64 × 64 pixels.
const TILE: u32 = 64;

const PROP_END: u32 = 0;
const PROP_OPACITY: u32 = 6;
const PROP_MODE: u32 = 7;
const PROP_VISIBLE: u32 = 8;
const PROP_OFFSETS: u32 = 15;
const PROP_COMPRESSION: u32 = 17;
const PROP_RESOLUTION: u32 = 19;
const PROP_FLOAT_OPACITY: u32 = 33;

/// What [`export`] writes.
#[derive(Clone, Debug)]
pub struct XcfOptions {
    /// Pixels per point.
    pub scale: f64,
    /// Pixels per inch stored in the file.
    pub ppi: f64,
    /// An opaque background colour as the bottom layer; `None`: transparent.
    pub background: Option<[u8; 3]>,
    /// One XCF layer per top-level layer (false: one flat layer).
    pub layers: bool,
    /// The renderer's options (anti-aliasing, templates left out).
    pub render: RenderOptions,
}

/// One XCF layer: straight RGBA pixels of `width × height` placed at `(left, top)`.
struct Layer {
    name: String,
    width: u32,
    height: u32,
    left: i32,
    top: i32,
    opacity: f32,
    visible: bool,
    mode: u32,
    pixels: Vec<u8>,
}

/// The XCF layer mode of a blend mode (the "default" layer modes of GIMP 2.10+).
fn mode_code(b: BlendMode) -> u32 {
    match b {
        BlendMode::Normal => 28,
        BlendMode::Darken => 35,
        BlendMode::Multiply => 30,
        BlendMode::ColorBurn => 43,
        BlendMode::Lighten => 36,
        BlendMode::Screen => 31,
        BlendMode::ColorDodge => 42,
        BlendMode::Overlay => 23,
        BlendMode::SoftLight => 45,
        BlendMode::HardLight => 44,
        BlendMode::Difference => 32,
        BlendMode::Exclusion => 52,
        BlendMode::Hue => 37,
        BlendMode::Saturation => 38,
        BlendMode::Color => 39,
        BlendMode::Luminosity => 56,
    }
}

fn check_size(w: u32, h: u32) -> Result<(), String> {
    if w == 0 || h == 0 {
        return Err("the export region is empty".into());
    }
    if w > MAX_SIDE || h > MAX_SIDE {
        return Err(format!("XCF images are at most {MAX_SIDE} pixels a side ({w} × {h} asked)"));
    }
    Ok(())
}

/// `region` of `doc` as an XCF file.
pub fn export(r: &mut Renderer, doc: &Document, region: Rect, o: &XcfOptions) -> Result<Vec<u8>, String> {
    let (width, height) = crate::region_pixels(region, o.scale);
    check_size(width, height)?;
    let transparent = RenderOptions { background: None, ..o.render.clone() };
    let mut layers: Vec<Layer> = vec![];
    if let Some([red, g, b]) = o.background {
        let mut px = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..width * height {
            px.extend_from_slice(&[red, g, b, 255]);
        }
        layers.push(Layer { name: "Background".into(), width, height, left: 0, top: 0, opacity: 1.0, visible: true, mode: 28, pixels: px });
    }
    if o.layers {
        let mut base = doc.clone();
        base.layers.clear();
        for l in &doc.layers {
            if matches!(l.kind, NodeKind::Layer { template: true, .. }) || !l.visible {
                continue;
            }
            if let Some(layer) = draw_part(r, &mut base, l, region, o.scale, width, height, &transparent) {
                layers.push(layer);
            }
        }
    } else {
        let img = r.render_region_with(doc, region, o.scale, &transparent);
        layers.push(Layer {
            name: if doc.title.is_empty() { "Layer".into() } else { doc.title.clone() },
            width: img.width,
            height: img.height,
            left: 0,
            top: 0,
            opacity: 1.0,
            visible: true,
            mode: 28,
            pixels: img.to_straight(),
        });
    }
    if layers.is_empty() {
        // An empty image still needs one layer for readers to show.
        layers.push(Layer {
            name: "Layer".into(),
            width,
            height,
            left: 0,
            top: 0,
            opacity: 1.0,
            visible: true,
            mode: 28,
            pixels: vec![0; (width * height * 4) as usize],
        });
    }
    Ok(write(width, height, o.ppi, &layers))
}

/// Top-level layer `n` drawn alone over `region`, cropped to what it paints.
fn draw_part(
    r: &mut Renderer,
    base: &mut Document,
    n: &Node,
    region: Rect,
    scale: f64,
    width: u32,
    height: u32,
    ro: &RenderOptions,
) -> Option<Layer> {
    let mut part = n.clone();
    (part.visible, part.opacity, part.blend) = (true, 1.0, BlendMode::Normal);
    let layer = if part.is_layer() {
        part
    } else {
        let mut l = Node::layer(base.alloc_id(), "", LayerColor::Preset(0));
        if let Some(children) = l.children_mut() {
            children.push(Arc::new(part));
        }
        l
    };
    base.layers = vec![Arc::new(layer)];
    let img = r.render_region_with(base, region, scale, ro);
    let px = img.to_straight();
    let (iw, ih) = (img.width.min(width) as usize, img.height.min(height) as usize);
    let painted = |x: usize, y: usize| px.get((y * img.width as usize + x) * 4 + 3).is_some_and(|a| *a > 0);
    let rows: Vec<usize> = (0..ih).filter(|y| (0..iw).any(|x| painted(x, *y))).collect();
    let (Some(&y0), Some(&y1)) = (rows.first(), rows.last()) else { return None };
    let cols: Vec<usize> = (0..iw).filter(|x| (y0..=y1).any(|y| painted(*x, y))).collect();
    let (Some(&x0), Some(&x1)) = (cols.first(), cols.last()) else { return None };
    let mut pixels = Vec::with_capacity((x1 - x0 + 1) * (y1 - y0 + 1) * 4);
    for y in y0..=y1 {
        let row = y * img.width as usize;
        pixels.extend_from_slice(px.get((row + x0) * 4..(row + x1 + 1) * 4).unwrap_or_default());
    }
    Some(Layer {
        name: n.display_name(),
        width: (x1 - x0 + 1) as u32,
        height: (y1 - y0 + 1) as u32,
        left: x0 as i32,
        top: y0 as i32,
        opacity: n.opacity.clamp(0.0, 1.0),
        visible: n.visible,
        mode: mode_code(n.blend),
        pixels,
    })
}

struct Out(Vec<u8>);

impl Out {
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    /// An XCF string: its length including the terminating NUL, the bytes, NUL.
    fn string(&mut self, s: &str) {
        let b = s.as_bytes();
        self.u32(b.len() as u32 + 1);
        self.0.extend_from_slice(b);
        self.0.push(0);
    }
    fn prop(&mut self, kind: u32, payload: &[u8]) {
        self.u32(kind);
        self.u32(payload.len() as u32);
        self.0.extend_from_slice(payload);
    }
    fn at(&self) -> u64 {
        self.0.len() as u64
    }
    fn patch_u64(&mut self, at: usize, v: u64) {
        if let Some(b) = self.0.get_mut(at..at + 8) {
            b.copy_from_slice(&v.to_be_bytes());
        }
    }
}

/// The file: header, image properties, the layer pointer list, then each layer with its
/// hierarchy, level and tiles (uncompressed RGBA).
fn write(width: u32, height: u32, ppi: f64, layers: &[Layer]) -> Vec<u8> {
    let mut o = Out(Vec::new());
    o.0.extend_from_slice(b"gimp xcf v011\0");
    o.u32(width);
    o.u32(height);
    o.u32(0); // RGB
    o.u32(150); // 8-bit gamma integer
    o.prop(PROP_COMPRESSION, &[0]);
    let res = ppi.clamp(1.0, 65536.0) as f32;
    let mut r = Out(Vec::new());
    r.f32(res);
    r.f32(res);
    o.prop(PROP_RESOLUTION, &r.0);
    o.prop(PROP_END, &[]);
    // Layer pointers (top layer first), then the terminator; channel list: empty.
    let pointers_at = o.0.len();
    for _ in layers {
        o.u64(0);
    }
    o.u64(0);
    o.u64(0);
    for (i, l) in layers.iter().rev().enumerate() {
        let here = o.at();
        o.patch_u64(pointers_at + i * 8, here);
        write_layer(&mut o, l);
    }
    o.0
}

fn write_layer(o: &mut Out, l: &Layer) {
    o.u32(l.width);
    o.u32(l.height);
    o.u32(1); // RGBA
    o.string(&l.name);
    let mut p = Out(Vec::new());
    p.u32((l.opacity * 255.0).round().clamp(0.0, 255.0) as u32);
    o.prop(PROP_OPACITY, &p.0);
    let mut p = Out(Vec::new());
    p.f32(l.opacity);
    o.prop(PROP_FLOAT_OPACITY, &p.0);
    let mut p = Out(Vec::new());
    p.u32(u32::from(l.visible));
    o.prop(PROP_VISIBLE, &p.0);
    let mut p = Out(Vec::new());
    p.u32(l.mode);
    o.prop(PROP_MODE, &p.0);
    let mut p = Out(Vec::new());
    p.i32(l.left);
    p.i32(l.top);
    o.prop(PROP_OFFSETS, &p.0);
    o.prop(PROP_END, &[]);
    // The hierarchy follows the two pointers (hierarchy, mask: none).
    let hier_ptr_at = o.0.len();
    o.u64(0);
    o.u64(0);
    let hier_at = o.at();
    o.patch_u64(hier_ptr_at, hier_at);
    o.u32(l.width);
    o.u32(l.height);
    o.u32(4); // bytes per pixel
    let level_ptr_at = o.0.len();
    o.u64(0);
    o.u64(0); // no lower levels
    let level_at = o.at();
    o.patch_u64(level_ptr_at, level_at);
    o.u32(l.width);
    o.u32(l.height);
    let cols = l.width.div_ceil(TILE);
    let rows = l.height.div_ceil(TILE);
    let tiles = (cols * rows) as usize;
    let tiles_ptr_at = o.0.len();
    for _ in 0..tiles {
        o.u64(0);
    }
    o.u64(0);
    for ty in 0..rows {
        for tx in 0..cols {
            let i = (ty * cols + tx) as usize;
            let here = o.at();
            o.patch_u64(tiles_ptr_at + i * 8, here);
            let x0 = tx * TILE;
            let y0 = ty * TILE;
            let tw = (l.width - x0).min(TILE);
            let th = (l.height - y0).min(TILE);
            for y in y0..y0 + th {
                let start = ((y * l.width + x0) * 4) as usize;
                let end = start + (tw * 4) as usize;
                match l.pixels.get(start..end) {
                    Some(row) => o.0.extend_from_slice(row),
                    None => o.0.extend(std::iter::repeat_n(0u8, (tw * 4) as usize)),
                }
            }
        }
    }
}
