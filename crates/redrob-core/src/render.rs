// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::sync::Arc;

use crate::precision::Precision;
use crate::{
    BlendMode, CoreError, Document, MAX_HIERARCHY_DEPTH, MAX_STORED_RASTER_BYTES, NodeId, NodeKind,
    Pixel, Rect, Result,
};

/// Maximum aggregate full-canvas pixel visits performed by one render.
///
/// The budget includes clearing the output, each visible raster composite, and
/// both clearing and compositing every visible isolated group. It permits a
/// largest-supported canvas with one raster nested in one group while bounding
/// amplification from valid wide trees.
pub const MAX_RENDER_PIXEL_VISITS: u64 = 256 * 1024 * 1024;

/// The region of the canvas a render has to recompute.
///
/// TRANSLATED CONCEPT from Krita's `KisBaseRectsWalker` / `KisMergeWalker`
/// (`libs/image/kis_base_rects_walker.h`, `libs/image/kis_merge_walker.cc`), GPL-2.0-or-later. Krita asks
/// every layer's projection plane to grow the requested rect twice over: `changeRect` for what the layer
/// alters, `needRect` for what it must READ to do so -- a blurring mask needs neighbours it does not change.
///
/// MEASURED here: this renderer has no image filters in its path and reads no neighbouring pixel, so
/// compositing is strictly per-pixel and the two rects are equal. That is a property of this product, not a
/// general truth, and it is why one rect is enough. A blur or a filter in the render path would need Krita's
/// second rect back, and `a_bounded_render_equals_a_full_render` is the test that would fail first.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Damage {
    /// Recompute the whole canvas. The DEFAULT for every command, deliberately: a command that forgets to
    /// declare its region renders slowly, which is a performance bug, where a command that declares too
    /// small a region renders a stale frame, which is a correctness bug. The failure has to be the cheap one.
    Everything,
    /// Recompute only this region.
    Region(Rect),
    /// Nothing is outstanding -- the projection is current.
    ///
    /// This has to be its own variant rather than a zero-sized `Region`. MEASURED: representing "nothing" as
    /// `Rect::new(0, 0, 0, 0)` made every union drag its result back to the canvas origin, because the union
    /// of an empty box AT THE ORIGIN with a dab at (400, 400) is the whole 405x405 corner. The damage looked
    /// correct, the frame was correct, and the region was 2,000 times too large -- a performance bug that no
    /// correctness test could see.
    Nothing,
}

impl Damage {
    pub(crate) fn union(self, other: Self) -> Self {
        match (self, other) {
            (Self::Everything, _) | (_, Self::Everything) => Self::Everything,
            (Self::Nothing, other) => other,
            (current, Self::Nothing) => current,
            (Self::Region(left), Self::Region(right)) => Self::Region(union_rect(left, right)),
        }
    }

    /// The pixel box this damage covers on a canvas of the given size, or `None` when it covers nothing.
    fn clipped(self, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
        match self {
            Self::Everything => (width > 0 && height > 0).then_some((0, 0, width, height)),
            Self::Region(rect) => rect.clipped_bounds(width, height),
            Self::Nothing => None,
        }
    }
}

pub(crate) fn union_rect(left: Rect, right: Rect) -> Rect {
    let x0 = left.x.min(right.x);
    let y0 = left.y.min(right.y);
    let x1 = (i64::from(left.x) + i64::from(left.width))
        .max(i64::from(right.x) + i64::from(right.width));
    let y1 = (i64::from(left.y) + i64::from(left.height))
        .max(i64::from(right.y) + i64::from(right.height));
    Rect::new(
        x0,
        y0,
        (x1 - i64::from(x0)).clamp(0, i64::from(u32::MAX)) as u32,
        (y1 - i64::from(y0)).clamp(0, i64::from(u32::MAX)) as u32,
    )
}

/// Immutable flattened pixels associated with an editor generation.
#[derive(Clone, Debug)]
pub struct RenderSnapshot {
    width: u32,
    height: u32,
    generation: u64,
    pixels: Arc<[u8]>,
}

struct Renderer<'a> {
    document: &'a Document,
    frame: crate::FrameId,
    children: HashMap<Option<NodeId>, Vec<usize>>,
    /// Rows and columns outside this box are left exactly as they were found, which is what makes a
    /// projection reusable between frames.
    bounds: (u32, u32, u32, u32),
}

impl Renderer<'_> {
    fn render_work_operations(&self, parent: Option<NodeId>, depth: usize) -> Result<u64> {
        let mut operations = 0_u64;
        for &index in self.children.get(&parent).into_iter().flatten() {
            operations = operations
                .checked_add(self.node_work_operations(index, depth)?)
                .ok_or(CoreError::RenderWorkLimitExceeded {
                    max_pixel_visits: MAX_RENDER_PIXEL_VISITS,
                })?;
        }
        Ok(operations)
    }

    fn node_work_operations(&self, index: usize, depth: usize) -> Result<u64> {
        if depth > MAX_HIERARCHY_DEPTH {
            return Err(CoreError::DocumentLimitExceeded("hierarchy depth"));
        }
        let node = &self.document.nodes()[index];
        if !node.is_visible() || node.opacity() <= 0.0 {
            return Ok(0);
        }
        match node.kind() {
            NodeKind::Raster => Ok(1),
            NodeKind::Text | NodeKind::Vector => {
                crate::semantic::preflight(
                    node.content(),
                    self.document.width(),
                    self.document.height(),
                )?;
                Ok(2)
            }
            NodeKind::Group => self
                .render_work_operations(Some(node.id()), depth + 1)?
                .checked_add(2)
                .ok_or(CoreError::RenderWorkLimitExceeded {
                    max_pixel_visits: MAX_RENDER_PIXEL_VISITS,
                }),
        }
    }

    fn preflight_render_work(&self, canvas_pixels: u64) -> Result<()> {
        let operations = self.render_work_operations(None, 0)?.checked_add(1).ok_or(
            CoreError::RenderWorkLimitExceeded {
                max_pixel_visits: MAX_RENDER_PIXEL_VISITS,
            },
        )?;
        let pixel_visits =
            canvas_pixels
                .checked_mul(operations)
                .ok_or(CoreError::RenderWorkLimitExceeded {
                    max_pixel_visits: MAX_RENDER_PIXEL_VISITS,
                })?;
        if pixel_visits > MAX_RENDER_PIXEL_VISITS {
            return Err(CoreError::RenderWorkLimitExceeded {
                max_pixel_visits: MAX_RENDER_PIXEL_VISITS,
            });
        }
        Ok(())
    }

    fn render_children(
        &self,
        parent: Option<NodeId>,
        destination: &mut [u8],
        depth: usize,
    ) -> Result<()> {
        for &index in self.children.get(&parent).into_iter().flatten() {
            self.render_node(index, destination, depth)?;
        }
        Ok(())
    }

    fn render_node(&self, index: usize, destination: &mut [u8], depth: usize) -> Result<()> {
        if depth > MAX_HIERARCHY_DEPTH {
            return Err(CoreError::DocumentLimitExceeded("hierarchy depth"));
        }
        let node = &self.document.nodes()[index];
        if !node.is_visible() || node.opacity() <= 0.0 {
            return Ok(());
        }
        match node.kind() {
            NodeKind::Raster => {
                let Ok(pixels) = node.raster_pixels(self.frame) else {
                    return Ok(());
                };
                // The compositor is 8-bit, and the document's samples may now be wider (J.1a). So
                // the render boundary is where a deep document is brought down to the display
                // depth. Passing the stored bytes straight through instead does not fail — it
                // reads each 16-bit sample as two 8-bit ones and draws the first quarter of the
                // image stretched over the canvas, with no error anywhere.
                //
                // Compositing AT the document's precision is J.1d. This is the conversion that
                // keeps the canvas correct until then, and it costs nothing at 8-bit, where
                // `convert` returns the same bytes.
                let converted;
                let pixels = if self.document.precision() == Precision::U8 {
                    pixels
                } else {
                    converted = self
                        .document
                        .precision()
                        .convert(pixels, Precision::U8)
                        .bytes;
                    &converted
                };
                composite_buffer(
                    destination,
                    pixels,
                    node.mask()
                        .filter(|mask| mask.is_enabled())
                        .map(|mask| mask.pixels()),
                    node.opacity(),
                    node.blend_mode(),
                    self.document.width(),
                    self.bounds,
                );
                Ok(())
            }
            NodeKind::Text | NodeKind::Vector => {
                let pixels = crate::semantic::rasterize(
                    node.content(),
                    self.document.width(),
                    self.document.height(),
                )?;
                composite_buffer(
                    destination,
                    &pixels,
                    None,
                    node.opacity(),
                    node.blend_mode(),
                    self.document.width(),
                    self.bounds,
                );
                Ok(())
            }
            NodeKind::Group => {
                // The intermediate stays full-canvas because indices are absolute, but only the bounded rows
                // are written into it and read back out, so its cost is the allocation rather than the area.
                // Krita avoids even that with a pooled paint device; a pool is its own change.
                let mut intermediate = vec![0_u8; destination.len()];
                self.render_children(Some(node.id()), &mut intermediate, depth + 1)?;
                composite_buffer(
                    destination,
                    &intermediate,
                    node.mask()
                        .filter(|mask| mask.is_enabled())
                        .map(|mask| mask.pixels()),
                    node.opacity(),
                    node.blend_mode(),
                    self.document.width(),
                    self.bounds,
                );
                Ok(())
            }
        }
    }
}

fn composite_buffer(
    destination: &mut [u8],
    source: &[u8],
    mask: Option<&[u8]>,
    opacity: f32,
    mode: BlendMode,
    width: u32,
    bounds: (u32, u32, u32, u32),
) {
    let (x0, y0, x1, y1) = bounds;
    let row_stride = width as usize;
    for y in y0..y1 {
        // Absolute indices, deliberately. A bounded pass and a full pass must address the same pixel for the
        // same coordinate, or a reused projection would be written at an offset -- and the equivalence test
        // is then the only thing between that and a shipped product.
        let row = y as usize * row_stride;
        for x in x0..x1 {
            let index = row + x as usize;
            let offset = index * 4;
            let mut source_pixel = Pixel::from_slice(&source[offset..offset + 4]);
            if let Some(mask) = mask {
                source_pixel.a =
                    ((u16::from(source_pixel.a) * u16::from(mask[index]) + 127) / 255) as u8;
            }
            let slot = &mut destination[offset..offset + 4];
            if matches!(mode, BlendMode::Dissolve) {
                // Dissolve turns partial coverage into a random scatter of fully-opaque pixels: a
                // pixel is painted iff a per-pixel hash falls under its coverage, then composited
                // Normal at full alpha. Deterministic in the pixel index, so a re-render is identical.
                let coverage = f32::from(source_pixel.a) / 255.0 * opacity.clamp(0.0, 1.0);
                let mut h = (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
                h ^= h >> 29;
                h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                h ^= h >> 32;
                let r = (h & 0xFFFF) as f32 / 65535.0;
                if r < coverage {
                    let opaque = Pixel::rgba(source_pixel.r, source_pixel.g, source_pixel.b, 255);
                    composite(Pixel::from_slice(slot), opaque, 1.0, BlendMode::Normal)
                        .write_to(slot);
                }
                continue;
            }
            composite(Pixel::from_slice(slot), source_pixel, opacity, mode).write_to(slot);
        }
    }
}

fn renderer_for(
    document: &Document,
    frame: crate::FrameId,
    bounds: (u32, u32, u32, u32),
) -> Renderer<'_> {
    let mut children = HashMap::<Option<NodeId>, Vec<usize>>::new();
    for (index, node) in document.nodes().iter().enumerate() {
        children.entry(node.parent_id()).or_default().push(index);
    }
    Renderer {
        document,
        frame,
        children,
        bounds,
    }
}

pub(crate) fn preflight_document(document: &Document) -> Result<()> {
    let pixel_bytes = (document.width() as usize)
        .checked_mul(document.height() as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(CoreError::DocumentLimitExceeded("render working bytes"))?;
    let max_group_depth = document
        .nodes()
        .iter()
        .filter(|node| node.kind() == NodeKind::Group)
        .filter_map(|node| document.node_depth(node.id()))
        .max()
        .map_or(0, |depth| depth + 1);
    let semantic_temporary = usize::from(
        document
            .nodes()
            .iter()
            .any(|node| matches!(node.kind(), NodeKind::Text | NodeKind::Vector)),
    );
    let working_bytes = pixel_bytes
        .checked_mul(
            max_group_depth
                .saturating_add(1)
                .saturating_add(semantic_temporary),
        )
        .ok_or(CoreError::DocumentLimitExceeded("render working bytes"))?;
    if working_bytes as u64 > MAX_STORED_RASTER_BYTES {
        return Err(CoreError::DocumentLimitExceeded("render working bytes"));
    }
    // The preflight counts the work a FULL render would do, deliberately: the budget must be the worst case,
    // not whatever this frame's damage happens to be, or a document would pass while being damaged narrowly
    // and fail later on a full repaint.
    renderer_for(
        document,
        document.current_frame_id(),
        (0, 0, document.width(), document.height()),
    )
    .preflight_render_work((pixel_bytes / 4) as u64)
}

impl RenderSnapshot {
    /// Renders one existing frame without changing document navigation or history.
    pub fn try_render_frame(
        document: &Document,
        generation: u64,
        frame: crate::FrameId,
    ) -> Result<Self> {
        document.validate()?;
        if !document.timeline().contains(frame) {
            return Err(CoreError::FrameNotFound(frame));
        }
        preflight_document(document)?;
        let pixel_bytes = (document.width() as usize)
            .checked_mul(document.height() as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(CoreError::DocumentLimitExceeded("render working bytes"))?;
        let renderer = renderer_for(document, frame, (0, 0, document.width(), document.height()));

        let mut output = vec![0_u8; pixel_bytes];
        renderer.render_children(None, &mut output, 0)?;
        Ok(Self {
            width: document.width(),
            height: document.height(),
            generation,
            pixels: output.into(),
        })
    }

    /// Recomposites only the damaged region, reusing a previous projection for everything outside it.
    ///
    /// TRANSLATED CONCEPT from Krita's `KisAsyncMerger` (`libs/image/kis_async_merger.cpp`),
    /// GPL-2.0-or-later. Krita keeps a projection paint device per layer and per image and its merger updates
    /// only the walker's rect; nothing else is recomputed and nothing is reallocated.
    ///
    /// MEASURED before this existed: every dab recomposited the whole canvas AND allocated a fresh
    /// full-canvas buffer. At 4000x4000 that was 142.7 ms of a 178.6 ms dab -- 80% of the latency -- and the
    /// bench's own notes already said what the fix would look like ("a flat median down the area sweep means
    /// only the touched region is recomputed").
    ///
    /// The reuse is COPY-ON-WRITE, which is Krita's paint-device semantics too: when the caller has dropped
    /// the previous snapshot the buffer is unshared and updated in place, and when the caller is still
    /// holding it the buffer is cloned once so their frame cannot change under them.
    pub(crate) fn try_render_damage(
        document: &Document,
        generation: u64,
        damage: Damage,
        previous: Option<Arc<[u8]>>,
    ) -> Result<Self> {
        document.validate()?;
        let frame = document.current_frame_id();
        preflight_document(document)?;
        let pixel_bytes = (document.width() as usize)
            .checked_mul(document.height() as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(CoreError::DocumentLimitExceeded("render working bytes"))?;

        // A projection can only be reused when it describes the same canvas. A resize leaves a buffer of the
        // wrong length, and reusing it would index outside the new canvas.
        let reusable = previous.filter(|pixels| pixels.len() == pixel_bytes);
        let Some(bounds) = damage.clipped(document.width(), document.height()) else {
            // Nothing visible was damaged. Hand back the projection unchanged rather than recomputing it.
            let pixels = match reusable {
                Some(pixels) => pixels,
                None => return Self::try_render_frame(document, generation, frame),
            };
            return Ok(Self {
                width: document.width(),
                height: document.height(),
                generation,
                pixels,
            });
        };
        let (mut pixels, bounds) = match reusable {
            Some(pixels) => (pixels, bounds),
            // With no projection to build on, every pixel outside the damage would be left uninitialised, so
            // the first frame after a resize or a load is necessarily a full render.
            None => (
                Arc::from(vec![0_u8; pixel_bytes]),
                (0, 0, document.width(), document.height()),
            ),
        };

        // Clear the damaged region before compositing into it: the previous frame's content is there, and
        // compositing over it would accumulate rather than replace. Outside the region the previous frame is
        // exactly what we want to keep, which is the whole point.
        let slot = match Arc::get_mut(&mut pixels) {
            Some(slot) => slot,
            None => {
                // The caller still holds the previous snapshot. Copy once so their frame stays as it was.
                pixels = Arc::from(pixels.to_vec());
                Arc::get_mut(&mut pixels).expect("freshly cloned Arc is unshared")
            }
        };
        let (x0, y0, x1, y1) = bounds;
        let row_stride = document.width() as usize;
        for y in y0..y1 {
            let row = y as usize * row_stride;
            slot[(row + x0 as usize) * 4..(row + x1 as usize) * 4].fill(0);
        }

        let renderer = renderer_for(document, frame, bounds);
        renderer.render_children(None, slot, 0)?;
        Ok(Self {
            width: document.width(),
            height: document.height(),
            generation,
            pixels,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Build a snapshot from an already-composited RGBA buffer (used by onion-skin rendering).
    pub(crate) fn from_pixels(
        width: u32,
        height: u32,
        generation: u64,
        pixels: Vec<u8>,
    ) -> Result<Self> {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or(CoreError::DocumentLimitExceeded("render working bytes"))?;
        if pixels.len() != expected {
            return Err(CoreError::InvalidBufferLength {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            generation,
            pixels: pixels.into(),
        })
    }

    pub fn shared_pixels(&self) -> Arc<[u8]> {
        Arc::clone(&self.pixels)
    }
}

/// Onion skin (GIMP/Krita animation): render `frame` with its neighbouring frames ghosted behind it.
/// `before`/`after` are how many previous/next frames to include; each is tinted (previous toward
/// `tint_before`, next toward `tint_after`) and faded by `opacity` falling off with distance, then
/// the current frame is composited on top at full strength. Re-derived from the onion-skin feature,
/// our own compositing.
// The onion-skin parameters are the user's own controls (how many frames each way, each tint, the
// ghost strength); a struct here would only be these fields under another name.
#[allow(clippy::too_many_arguments)]
pub fn render_onion_skin(
    document: &Document,
    generation: u64,
    frame: crate::FrameId,
    before: u32,
    after: u32,
    tint_before: Pixel,
    tint_after: Pixel,
    opacity: f32,
) -> Result<RenderSnapshot> {
    let frames = document.timeline().frames();
    let index = document
        .timeline()
        .frame_index(frame)
        .ok_or(CoreError::FrameNotFound(frame))?;
    let width = document.width();
    let height = document.height();
    let n = (width as usize) * (height as usize);
    let mut canvas = vec![0u8; n * 4];
    let base_opacity = opacity.clamp(0.0, 1.0);

    // Collect ghost layers: previous frames first (so the nearest previous is on top of older), then
    // next frames, then the current frame last on top.
    let mut ghosts: Vec<(usize, Pixel, f32)> = Vec::new();
    for step in (1..=before).rev() {
        if let Some(i) = index.checked_sub(step as usize) {
            let fade = base_opacity * (1.0 - step as f32 / (before as f32 + 1.0));
            ghosts.push((i, tint_before, fade));
        }
    }
    for step in (1..=after).rev() {
        let i = index + step as usize;
        if i < frames.len() {
            let fade = base_opacity * (1.0 - step as f32 / (after as f32 + 1.0));
            ghosts.push((i, tint_after, fade));
        }
    }
    // Nearest ghosts should sit closest to the current frame; sort so smaller distance composites later.
    ghosts.sort_by_key(|&(i, _, _)| (index as isize - i as isize).unsigned_abs());
    // Composite farthest first.
    for &(gi, tint, fade) in ghosts.iter().rev() {
        let ghost = RenderSnapshot::try_render_frame(document, generation, frames[gi].id())?;
        let gp = ghost.pixels();
        for px in 0..n {
            let o = px * 4;
            // Tint the ghost toward its colour, then fade its alpha.
            let mut src = Pixel::rgba(
                ((u16::from(gp[o]) + u16::from(tint.r)) / 2) as u8,
                ((u16::from(gp[o + 1]) + u16::from(tint.g)) / 2) as u8,
                ((u16::from(gp[o + 2]) + u16::from(tint.b)) / 2) as u8,
                gp[o + 3],
            );
            src.a = ((f32::from(src.a) * fade).round().clamp(0.0, 255.0)) as u8;
            let dst = Pixel::from_slice(&canvas[o..o + 4]);
            source_over(dst, src).write_to(&mut canvas[o..o + 4]);
        }
    }
    // The current frame on top at full strength.
    let current = RenderSnapshot::try_render_frame(document, generation, frame)?;
    let cp = current.pixels();
    for px in 0..n {
        let o = px * 4;
        let src = Pixel::from_slice(&cp[o..o + 4]);
        let dst = Pixel::from_slice(&canvas[o..o + 4]);
        source_over(dst, src).write_to(&mut canvas[o..o + 4]);
    }
    RenderSnapshot::from_pixels(width, height, generation, canvas)
}

pub(crate) fn source_over(destination: Pixel, source: Pixel) -> Pixel {
    composite(destination, source, 1.0, BlendMode::Normal)
}

// --- Non-separable blend modes (A.5) ------------------------------------------------------------
//
// These recombine colour components across the two layers instead of blending channel by channel.
// `d` is the destination (base), `s` is the source (blend layer), both as [r, g, b] in 0..=1.
//
// The HSV family and HSL color follow the W3C compositing non-separable helpers (the same ones GIMP
// uses): lum() is Rec. 601 luma, sat() is max-min, and set_lum / set_sat rebuild a colour with a
// target luma or saturation while keeping the rest. The LCH family and Luminance work in CIELAB so
// hue/chroma/lightness are perceptual, matching GIMP's LCH_* ops.

fn w3c_lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn w3c_clip_color(mut c: [f32; 3]) -> [f32; 3] {
    let l = w3c_lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    if n < 0.0 {
        for v in &mut c {
            *v = l + (*v - l) * l / (l - n);
        }
    }
    if x > 1.0 {
        for v in &mut c {
            *v = l + (*v - l) * (1.0 - l) / (x - l);
        }
    }
    c
}

fn w3c_set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - w3c_lum(c);
    w3c_clip_color([c[0] + d, c[1] + d, c[2] + d])
}

fn w3c_sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

// Set the saturation of `c` to `s` while keeping its luma relationship, per the W3C mid/min/max rule.
fn w3c_set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let mut idx = [0usize, 1, 2];
    idx.sort_by(|&a, &b| c[a].partial_cmp(&c[b]).unwrap_or(std::cmp::Ordering::Equal));
    let (lo, mid, hi) = (idx[0], idx[1], idx[2]);
    let mut out = [0.0_f32; 3];
    if c[hi] > c[lo] {
        out[mid] = (c[mid] - c[lo]) * s / (c[hi] - c[lo]);
        out[hi] = s;
    }
    out[lo] = 0.0;
    out
}

// sRGB <-> CIELAB (D65), used by the LCH modes and perceptual Luminance.
fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

fn rgb_to_lab(c: [f32; 3]) -> [f32; 3] {
    let r = srgb_to_linear(c[0]);
    let g = srgb_to_linear(c[1]);
    let b = srgb_to_linear(c[2]);
    // Linear sRGB -> XYZ (D65), normalised by the white point.
    let x = (0.412_453 * r + 0.357_580 * g + 0.180_423 * b) / 0.950_456;
    let y = 0.212_671 * r + 0.715_160 * g + 0.072_169 * b;
    let z = (0.019_334 * r + 0.119_193 * g + 0.950_227 * b) / 1.088_754;
    let f = |t: f32| {
        if t > 0.008_856 {
            t.cbrt()
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn lab_to_rgb(lab: [f32; 3]) -> [f32; 3] {
    let fy = (lab[0] + 16.0) / 116.0;
    let fx = fy + lab[1] / 500.0;
    let fz = fy - lab[2] / 200.0;
    let g = |t: f32| {
        let t3 = t * t * t;
        if t3 > 0.008_856 {
            t3
        } else {
            (t - 16.0 / 116.0) / 7.787
        }
    };
    let x = g(fx) * 0.950_456;
    let y = g(fy);
    let z = g(fz) * 1.088_754;
    let r = 3.240_479 * x - 1.537_15 * y - 0.498_535 * z;
    let gg = -0.969_256 * x + 1.875_992 * y + 0.041_556 * z;
    let b = 0.055_648 * x - 0.204_043 * y + 1.057_311 * z;
    [
        linear_to_srgb(r).clamp(0.0, 1.0),
        linear_to_srgb(gg).clamp(0.0, 1.0),
        linear_to_srgb(b).clamp(0.0, 1.0),
    ]
}

fn nonseparable_blend(mode: BlendMode, d: [f32; 3], s: [f32; 3]) -> [f32; 3] {
    match mode {
        // HSV_HUE: source hue, destination saturation and value -> set_lum(set_sat(s, sat(d)), lum(d)).
        BlendMode::HsvHue => w3c_set_lum(w3c_set_sat(s, w3c_sat(d)), w3c_lum(d)),
        // HSV_SATURATION: source saturation, destination hue and value.
        BlendMode::HsvSaturation => w3c_set_lum(w3c_set_sat(d, w3c_sat(s)), w3c_lum(d)),
        // HSV_VALUE / Luminosity: source luma, destination hue and saturation.
        BlendMode::HsvValue => w3c_set_lum(d, w3c_lum(s)),
        // HSL color: source hue and saturation, destination luma.
        BlendMode::HslColor => w3c_set_lum(s, w3c_lum(d)),
        // LCH family in CIELAB: swap one of L, C(=hypot(a,b)), H(=atan2(b,a)).
        BlendMode::LchHue => {
            let dl = rgb_to_lab(d);
            let sl = rgb_to_lab(s);
            let dc = (dl[1] * dl[1] + dl[2] * dl[2]).sqrt();
            let sh = sl[2].atan2(sl[1]);
            lab_to_rgb([dl[0], dc * sh.cos(), dc * sh.sin()])
        }
        BlendMode::LchChroma => {
            let dl = rgb_to_lab(d);
            let sl = rgb_to_lab(s);
            let sc = (sl[1] * sl[1] + sl[2] * sl[2]).sqrt();
            let dh = dl[2].atan2(dl[1]);
            lab_to_rgb([dl[0], sc * dh.cos(), sc * dh.sin()])
        }
        // LCH color: source chroma and hue (a,b), destination lightness.
        BlendMode::LchColor => {
            let dl = rgb_to_lab(d);
            let sl = rgb_to_lab(s);
            lab_to_rgb([dl[0], sl[1], sl[2]])
        }
        // LCH lightness: source lightness, destination chroma and hue.
        BlendMode::LchLightness => {
            let dl = rgb_to_lab(d);
            let sl = rgb_to_lab(s);
            lab_to_rgb([sl[0], dl[1], dl[2]])
        }
        // Luminance: destination colour at the source's perceptual lightness.
        BlendMode::Luminance => {
            let dl = rgb_to_lab(d);
            let sl = rgb_to_lab(s);
            lab_to_rgb([sl[0], dl[1], dl[2]])
        }
        _ => s,
    }
}

fn composite(destination: Pixel, source: Pixel, opacity: f32, mode: BlendMode) -> Pixel {
    // Composite ops (A.6) act on alpha and order rather than through the colour formula, so they are
    // resolved before the standard source-over blend. Dissolve is handled in composite_buffer (it
    // needs the pixel coordinate); PassThrough falls through to Normal here (a pixel-level no-op).
    let eff = (f32::from(source.a) / 255.0 * opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    match mode {
        // Source goes under the destination: destination-over.
        BlendMode::Behind => {
            let under = Pixel::rgba(source.r, source.g, source.b, eff);
            return composite(under, destination, 1.0, BlendMode::Normal);
        }
        // Copy the source, including its alpha, ignoring the destination entirely.
        BlendMode::Replace | BlendMode::Overwrite => {
            return Pixel::rgba(source.r, source.g, source.b, eff);
        }
        // Erase: subtract the source's coverage from the destination's alpha, colour kept.
        BlendMode::Erase => {
            let keep = (f32::from(destination.a) * (1.0 - f32::from(eff) / 255.0)).round() as u8;
            return Pixel::rgba(destination.r, destination.g, destination.b, keep);
        }
        // Anti-erase: add coverage back, bounded by full opacity (undoes an erase on a kept layer).
        BlendMode::AntiErase => {
            let add = f32::from(destination.a)
                + f32::from(eff) * (1.0 - f32::from(destination.a) / 255.0);
            return Pixel::rgba(
                destination.r,
                destination.g,
                destination.b,
                add.round().min(255.0) as u8,
            );
        }
        // Colour-erase: erase in proportion to how close the destination colour is to the source
        // colour (GIMP COLOR_ERASE) — the nearer the colour, the more alpha is removed.
        BlendMode::ColorErase => {
            let dr = (f32::from(destination.r) - f32::from(source.r)).abs() / 255.0;
            let dg = (f32::from(destination.g) - f32::from(source.g)).abs() / 255.0;
            let db = (f32::from(destination.b) - f32::from(source.b)).abs() / 255.0;
            let distance = dr.max(dg).max(db); // 0 = identical colour, 1 = opposite
            let removed = (f32::from(eff) / 255.0) * (1.0 - distance);
            let keep = (f32::from(destination.a) * (1.0 - removed)).round() as u8;
            return Pixel::rgba(destination.r, destination.g, destination.b, keep);
        }
        _ => {}
    }
    let source_alpha = f32::from(source.a) / 255.0 * opacity.clamp(0.0, 1.0);
    let destination_alpha = f32::from(destination.a) / 255.0;
    let output_alpha = source_alpha + destination_alpha - source_alpha * destination_alpha;
    if output_alpha <= f32::EPSILON {
        return Pixel::TRANSPARENT;
    }

    let source_channels = [source.r, source.g, source.b].map(|value| f32::from(value) / 255.0);
    let destination_channels =
        [destination.r, destination.g, destination.b].map(|value| f32::from(value) / 255.0);
    // Blend space (A.7). GIMP composites a handful of modes in LINEAR light by default (Multiply,
    // Addition, Subtract, Divide), the rest in perceptual sRGB. We follow the same per-mode default:
    // convert both colours to linear before the per-channel formula and back afterwards, so e.g. a
    // 50% grey multiplied by itself darkens the way GIMP's does rather than the sRGB way.
    let linear_space = matches!(
        mode,
        BlendMode::Multiply | BlendMode::Add | BlendMode::Subtract | BlendMode::Divide
    );
    let to_space = |c: [f32; 3]| {
        if linear_space {
            [
                srgb_to_linear(c[0]),
                srgb_to_linear(c[1]),
                srgb_to_linear(c[2]),
            ]
        } else {
            c
        }
    };
    let source_channels = to_space(source_channels);
    let destination_channels = to_space(destination_channels);
    // The Luma modes pick one whole pixel over the other by its Rec. 709 luma, rather than blending
    // channel by channel, so the chosen side's colour is kept intact (GIMP
    // gimpoperationlayermode-blend.c LUMA_DARKEN/LIGHTEN).
    let luma_pick: Option<[f32; 3]> = match mode {
        BlendMode::LumaDarkenOnly | BlendMode::LumaLightenOnly => {
            let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
            let source_wins = if matches!(mode, BlendMode::LumaDarkenOnly) {
                luma(source_channels) <= luma(destination_channels)
            } else {
                luma(source_channels) >= luma(destination_channels)
            };
            Some(if source_wins {
                source_channels
            } else {
                destination_channels
            })
        }
        // The non-separable colour modes recombine whole pixels; resolve the blended colour once.
        BlendMode::HsvHue
        | BlendMode::HsvSaturation
        | BlendMode::HsvValue
        | BlendMode::HslColor
        | BlendMode::LchHue
        | BlendMode::LchChroma
        | BlendMode::LchColor
        | BlendMode::LchLightness
        | BlendMode::Luminance => Some(nonseparable_blend(
            mode,
            destination_channels,
            source_channels,
        )),
        _ => None,
    };
    let mut output = [0_u8; 3];
    for channel in 0..3 {
        let source_value = source_channels[channel];
        let destination_value = destination_channels[channel];
        let blended = match mode {
            BlendMode::Normal => source_value,
            BlendMode::Multiply => source_value * destination_value,
            BlendMode::Screen => 1.0 - (1.0 - source_value) * (1.0 - destination_value),
            BlendMode::Overlay => {
                if destination_value <= 0.5 {
                    2.0 * source_value * destination_value
                } else {
                    1.0 - 2.0 * (1.0 - source_value) * (1.0 - destination_value)
                }
            }
            BlendMode::Add => (source_value + destination_value).min(1.0),
            BlendMode::DarkenOnly => source_value.min(destination_value),
            BlendMode::LightenOnly => source_value.max(destination_value),
            // Dodge/burn and the light family, GIMP gimpoperationlayermode-blend.c. s is source
            // ("blend" layer), d is destination ("base"). Guards avoid divide-by-zero at the ends.
            BlendMode::Dodge => {
                if source_value >= 1.0 {
                    1.0
                } else {
                    (destination_value / (1.0 - source_value)).min(1.0)
                }
            }
            BlendMode::Burn => {
                if source_value <= 0.0 {
                    0.0
                } else {
                    1.0 - ((1.0 - destination_value) / source_value).min(1.0)
                }
            }
            BlendMode::LinearBurn => (destination_value + source_value - 1.0).clamp(0.0, 1.0),
            BlendMode::LinearLight => {
                (destination_value + 2.0 * source_value - 1.0).clamp(0.0, 1.0)
            }
            BlendMode::VividLight => {
                if source_value <= 0.5 {
                    let d = 1.0 - 2.0 * source_value;
                    if d <= 0.0 {
                        0.0
                    } else {
                        1.0 - ((1.0 - destination_value) / d).min(1.0)
                    }
                } else {
                    let d = 2.0 * (1.0 - source_value);
                    if d <= 0.0 {
                        1.0
                    } else {
                        (destination_value / d).min(1.0)
                    }
                }
            }
            BlendMode::PinLight => {
                let twice = 2.0 * source_value;
                if source_value > 0.5 {
                    destination_value.max(twice - 1.0)
                } else {
                    destination_value.min(twice)
                }
            }
            BlendMode::HardMix => {
                // Vivid light taken to black or white: 1 where their sum reaches 1, else 0.
                if source_value + destination_value >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            // Hard light is overlay with source and destination swapped (GIMP HARDLIGHT).
            BlendMode::HardLight => {
                if source_value <= 0.5 {
                    2.0 * source_value * destination_value
                } else {
                    1.0 - 2.0 * (1.0 - source_value) * (1.0 - destination_value)
                }
            }
            // Soft light, the GIMP/W3C formula with the piecewise D(d).
            BlendMode::SoftLight => {
                let d = if destination_value <= 0.25 {
                    ((16.0 * destination_value - 12.0) * destination_value + 4.0)
                        * destination_value
                } else {
                    destination_value.sqrt()
                };
                if source_value <= 0.5 {
                    destination_value
                        - (1.0 - 2.0 * source_value) * destination_value * (1.0 - destination_value)
                } else {
                    destination_value + (2.0 * source_value - 1.0) * (d - destination_value)
                }
            }
            BlendMode::GrainExtract => (destination_value - source_value + 0.5).clamp(0.0, 1.0),
            BlendMode::GrainMerge => (destination_value + source_value - 0.5).clamp(0.0, 1.0),
            BlendMode::Difference => (destination_value - source_value).abs(),
            BlendMode::Exclusion => {
                destination_value + source_value - 2.0 * destination_value * source_value
            }
            BlendMode::Subtract => (destination_value - source_value).max(0.0),
            BlendMode::Divide => {
                if source_value <= 0.0 {
                    1.0
                } else {
                    (destination_value / source_value).min(1.0)
                }
            }
            BlendMode::LumaDarkenOnly | BlendMode::LumaLightenOnly => {
                luma_pick.map_or(source_value, |picked| picked[channel])
            }
            BlendMode::HsvHue
            | BlendMode::HsvSaturation
            | BlendMode::HsvValue
            | BlendMode::HslColor
            | BlendMode::LchHue
            | BlendMode::LchChroma
            | BlendMode::LchColor
            | BlendMode::LchLightness
            | BlendMode::Luminance => luma_pick.map_or(source_value, |picked| picked[channel]),
            // Dissolve and PassThrough use the Normal colour here; Dissolve's alpha is decided in
            // composite_buffer, PassThrough is a group flag. The alpha-order ops returned above.
            BlendMode::Dissolve
            | BlendMode::PassThrough
            | BlendMode::Behind
            | BlendMode::Erase
            | BlendMode::AntiErase
            | BlendMode::ColorErase
            | BlendMode::Replace
            | BlendMode::Overwrite => source_value,
        };
        let premultiplied = (1.0 - source_alpha) * destination_value * destination_alpha
            + (1.0 - destination_alpha) * source_value * source_alpha
            + source_alpha * destination_alpha * blended;
        // The straight (un-premultiplied) colour, converted back from linear if we blended there.
        let straight = (premultiplied / output_alpha).clamp(0.0, 1.0);
        let straight = if linear_space {
            linear_to_srgb(straight)
        } else {
            straight
        };
        output[channel] = (straight * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    Pixel::rgba(
        output[0],
        output[1],
        output[2],
        (output_alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_over_handles_translucent_pixels() {
        let result = source_over(Pixel::rgba(0, 0, 255, 255), Pixel::rgba(255, 0, 0, 128));
        assert_eq!(result, Pixel::rgba(128, 0, 127, 255));
    }

    #[test]
    fn darken_and_lighten_only_pick_per_channel_extremes() {
        // Opaque source over opaque destination: the alpha math is the identity, so the result is the
        // blended colour. Darken keeps the smaller channel, lighten the larger (GIMP DARKEN/LIGHTEN).
        let dst = Pixel::rgba(200, 50, 100, 255);
        let src = Pixel::rgba(100, 150, 100, 255);
        assert_eq!(
            composite(dst, src, 1.0, BlendMode::DarkenOnly),
            Pixel::rgba(100, 50, 100, 255)
        );
        assert_eq!(
            composite(dst, src, 1.0, BlendMode::LightenOnly),
            Pixel::rgba(200, 150, 100, 255)
        );
    }

    #[test]
    fn luma_modes_pick_the_whole_pixel_by_luma() {
        // Source luma 0.7152 (pure green) vs destination luma 0.2126 (pure red). Luma-darken keeps the
        // darker whole pixel (red), luma-lighten the lighter (green) -- channels are never mixed.
        let dst = Pixel::rgba(255, 0, 0, 255);
        let src = Pixel::rgba(0, 255, 0, 255);
        assert_eq!(
            composite(dst, src, 1.0, BlendMode::LumaDarkenOnly),
            Pixel::rgba(255, 0, 0, 255)
        );
        assert_eq!(
            composite(dst, src, 1.0, BlendMode::LumaLightenOnly),
            Pixel::rgba(0, 255, 0, 255)
        );
    }

    #[test]
    fn dodge_burn_and_light_family_match_the_formulas() {
        let at = |mode, s: u8, d: u8| {
            composite(
                Pixel::rgba(d, d, d, 255),
                Pixel::rgba(s, s, s, 255),
                1.0,
                mode,
            )
            .r
        };
        // Dodge d/(1-s): 128/255 over 102/255 -> 0.803 -> 205. Source 255 pins to white.
        assert_eq!(at(BlendMode::Dodge, 128, 102), 205);
        assert_eq!(at(BlendMode::Dodge, 255, 10), 255);
        // Burn 1-(1-d)/s: 128/255 over 153/255 -> 0.203 -> 52. Source 0 pins to black.
        assert_eq!(at(BlendMode::Burn, 128, 153), 52);
        assert_eq!(at(BlendMode::Burn, 0, 200), 0);
        // Linear burn d+s-1: 0.6+0.6-1 = 0.2 -> 51.
        assert_eq!(at(BlendMode::LinearBurn, 153, 153), 51);
        // Linear light d+2s-1: 0.25+2*0.251-1 clamps to 0; 0.749+2*0.502-1 = 0.753 -> 192.
        assert_eq!(at(BlendMode::LinearLight, 64, 64), 0);
        assert_eq!(at(BlendMode::LinearLight, 128, 191), 192);
        // Pin light: s>0.5 picks max(d, 2s-1); s<0.5 picks min(d, 2s).
        assert_eq!(at(BlendMode::PinLight, 230, 10), 205); // 2*0.902-1 = 0.804 > d
        assert_eq!(at(BlendMode::PinLight, 25, 230), 50); // 2*0.098 = 0.196 < d
        // Hard mix is black or white by whether s+d reaches 1.
        assert_eq!(at(BlendMode::HardMix, 100, 100), 0);
        assert_eq!(at(BlendMode::HardMix, 150, 150), 255);
        // Vivid light, s>0.5: d/(2(1-s)). 0.3/(2*0.1) = 1.0 -> 255.
        assert_eq!(at(BlendMode::VividLight, 230, 77), 255);
    }

    #[test]
    fn contrast_and_grain_modes_match_the_formulas() {
        let at = |mode, s: u8, d: u8| {
            composite(
                Pixel::rgba(d, d, d, 255),
                Pixel::rgba(s, s, s, 255),
                1.0,
                mode,
            )
            .r
        };
        // Hard light is overlay with the layers swapped: s<=0.5 -> 2sd, else screen.
        // s=0.25 (<=0.5), d=0.6: 2*0.25*0.6 = 0.3 -> 77.
        assert_eq!(at(BlendMode::HardLight, 64, 153), 77);
        // Soft light, s<0.5 darkens toward d*(1 - (1-2s)(1-d)). s=0 (full dark side):
        // d - 1*d*(1-d), d=0.6 -> 0.6-0.24 = 0.36 -> 92.
        assert_eq!(at(BlendMode::SoftLight, 0, 153), 92);
        // Grain extract d-s+0.5: 153/255 - 102/255 + 0.5 = 0.7 -> 179; clamps below 0.
        assert_eq!(at(BlendMode::GrainExtract, 102, 153), 179);
        assert_eq!(at(BlendMode::GrainExtract, 255, 0), 0);
        // Grain merge d+s-0.5: 102/255 + 153/255 - 0.5 = 0.5 -> 128; clamps above 1.
        assert_eq!(at(BlendMode::GrainMerge, 153, 102), 128);
        assert_eq!(at(BlendMode::GrainMerge, 255, 255), 255);
    }

    #[test]
    fn arithmetic_modes_match_the_formulas() {
        let at = |mode, s: u8, d: u8| {
            composite(
                Pixel::rgba(d, d, d, 255),
                Pixel::rgba(s, s, s, 255),
                1.0,
                mode,
            )
            .r
        };
        // Difference |d-s|: |0.6-0.2| = 0.4 -> 102.
        assert_eq!(at(BlendMode::Difference, 51, 153), 102);
        // Exclusion d+s-2ds: 0.6+0.2-2*0.6*0.2 = 0.56 -> 143.
        assert_eq!(at(BlendMode::Exclusion, 51, 153), 143);
        // Subtract and Divide blend in LINEAR light (A.7), so the sRGB inputs convert first.
        // Subtract max(d-s,0): 153 over 51 -> 146; floors at 0.
        assert_eq!(at(BlendMode::Subtract, 51, 153), 146);
        assert_eq!(at(BlendMode::Subtract, 200, 50), 0);
        // Divide min(d/s,1) in linear: 102/204 -> 129; source 0 pins to white.
        assert_eq!(at(BlendMode::Divide, 204, 102), 129);
        assert_eq!(at(BlendMode::Divide, 0, 50), 255);
    }

    #[test]
    fn nonseparable_modes_recombine_colour_components() {
        let c = |mode, s: Pixel, d: Pixel| composite(d, s, 1.0, mode);
        // HSV value / luminosity: destination hue+sat at the source's luma. Grey source over a red
        // destination keeps the hue red but takes the grey's brightness.
        let out = c(
            BlendMode::HsvValue,
            Pixel::rgba(128, 128, 128, 255),
            Pixel::rgba(255, 0, 0, 255),
        );
        assert!(out.r > out.g && out.r > out.b, "stays reddish: {out:?}");
        assert!(out.g == out.b, "grey source adds no colour cast: {out:?}");
        // HSL color: source hue+sat, destination luma. A saturated blue source over mid grey yields
        // a blue of the grey's lightness -- blue is the max channel.
        let out = c(
            BlendMode::HslColor,
            Pixel::rgba(0, 0, 255, 255),
            Pixel::rgba(128, 128, 128, 255),
        );
        assert!(
            out.b >= out.r && out.b >= out.g,
            "takes the blue hue: {out:?}"
        );
        // Luminance in CIELAB: identical source and destination is the identity (within rounding).
        let base = Pixel::rgba(120, 80, 200, 255);
        let out = c(BlendMode::Luminance, base, base);
        for (a, b) in [(out.r, base.r), (out.g, base.g), (out.b, base.b)] {
            assert!((a as i32 - b as i32).abs() <= 2, "round-trip {a} vs {b}");
        }
        // LCH hue takes the source hue at the destination lightness+chroma; swapping a grey source
        // (no chroma, undefined hue) leaves a near-grey.
        let out = c(
            BlendMode::LchHue,
            Pixel::rgba(128, 128, 128, 255),
            Pixel::rgba(200, 50, 50, 255),
        );
        assert!(out.a == 255, "opaque result: {out:?}");
    }

    #[test]
    fn composite_ops_act_on_alpha_and_order() {
        let red = Pixel::rgba(200, 40, 40, 255);
        let blue = Pixel::rgba(40, 40, 200, 255);
        // Behind: the source goes under, so an opaque destination is unchanged.
        assert_eq!(composite(red, blue, 1.0, BlendMode::Behind), red);
        // Replace: copy the source over anything.
        assert_eq!(composite(red, blue, 1.0, BlendMode::Replace), blue);
        // Erase: a full-opacity source clears the destination's alpha, colour kept.
        let erased = composite(red, Pixel::rgba(0, 0, 0, 255), 1.0, BlendMode::Erase);
        assert_eq!(erased, Pixel::rgba(200, 40, 40, 0));
        // Half-opacity erase halves the alpha.
        let half = composite(red, Pixel::rgba(0, 0, 0, 128), 1.0, BlendMode::Erase);
        assert!((126..=129).contains(&half.a), "half erase {}", half.a);
        // Colour-erase removes more where the colours match: erasing red with red clears it, erasing
        // red with blue (far colour) barely touches it.
        let same = composite(
            red,
            Pixel::rgba(200, 40, 40, 255),
            1.0,
            BlendMode::ColorErase,
        );
        assert_eq!(same.a, 0, "identical colour fully erased");
        let far = composite(
            red,
            Pixel::rgba(40, 40, 200, 255),
            1.0,
            BlendMode::ColorErase,
        );
        assert!(
            far.a > same.a + 100,
            "far colour erased less than near: {} vs {}",
            far.a,
            same.a
        );
    }

    #[test]
    fn multiply_blends_in_linear_light() {
        // 50%-ish sRGB grey (188) multiplied by itself. In linear light 0.5*0.5 = 0.25 linear,
        // ~137 in sRGB; the naive sRGB product would stay far brighter. A.7's point: the linear
        // result is darker and matches GIMP's default Multiply.
        let grey = Pixel::rgba(188, 188, 188, 255);
        let out = composite(grey, grey, 1.0, BlendMode::Multiply);
        assert!((132..=142).contains(&out.r), "linear multiply {}", out.r);
    }

    /// "Nothing damaged" must be the IDENTITY of the union, and a zero-sized rect is not.
    ///
    /// This is the bug this test exists for. Representing a cleared projection as `Rect::new(0, 0, 0, 0)`
    /// made the next union span from the canvas origin to the far corner of whatever was actually damaged: a
    /// dab at (400, 400) produced a 405x405 region instead of a 10x10 one. Every frame was CORRECT, so no
    /// equivalence test could fail -- only the measurement showed it, and only because the render stayed
    /// proportional to canvas area after it was supposed to be flat.
    #[test]
    fn nothing_is_the_identity_of_a_damage_union() {
        let dab = Rect::new(395, 395, 10, 10);
        assert_eq!(
            Damage::Nothing.union(Damage::Region(dab)),
            Damage::Region(dab),
            "a cleared projection must not widen the next region"
        );
        assert_eq!(
            Damage::Region(dab).union(Damage::Nothing),
            Damage::Region(dab),
            "in either order"
        );

        // The trap, stated as an assertion so the reason survives: an empty box AT THE ORIGIN is not nothing.
        let empty_at_origin = Rect::new(0, 0, 0, 0);
        let widened = union_rect(empty_at_origin, dab);
        assert_eq!(
            (widened.x, widened.y, widened.width, widened.height),
            (0, 0, 405, 405),
            "which is exactly why Nothing has to be its own variant"
        );
    }

    #[test]
    fn everything_absorbs_any_other_damage() {
        let dab = Rect::new(4, 4, 2, 2);
        assert_eq!(
            Damage::Everything.union(Damage::Region(dab)),
            Damage::Everything
        );
        assert_eq!(
            Damage::Region(dab).union(Damage::Everything),
            Damage::Everything
        );
        assert_eq!(
            Damage::Nothing.union(Damage::Everything),
            Damage::Everything
        );
    }

    #[test]
    fn nothing_clips_to_no_region_and_everything_clips_to_the_canvas() {
        assert_eq!(Damage::Nothing.clipped(64, 64), None);
        assert_eq!(Damage::Everything.clipped(64, 48), Some((0, 0, 64, 48)));
        // A region reaching outside the canvas is cropped to it, never allowed to index past the buffer.
        assert_eq!(
            Damage::Region(Rect::new(-10, 60, 100, 100)).clipped(64, 64),
            Some((0, 60, 64, 64))
        );
        // And one entirely outside covers nothing.
        assert_eq!(
            Damage::Region(Rect::new(200, 200, 10, 10)).clipped(64, 64),
            None
        );
    }
}
