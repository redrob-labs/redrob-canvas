// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::sync::Arc;

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

    pub fn shared_pixels(&self) -> Arc<[u8]> {
        Arc::clone(&self.pixels)
    }
}

pub(crate) fn source_over(destination: Pixel, source: Pixel) -> Pixel {
    composite(destination, source, 1.0, BlendMode::Normal)
}

fn composite(destination: Pixel, source: Pixel, opacity: f32, mode: BlendMode) -> Pixel {
    let source_alpha = f32::from(source.a) / 255.0 * opacity.clamp(0.0, 1.0);
    let destination_alpha = f32::from(destination.a) / 255.0;
    let output_alpha = source_alpha + destination_alpha - source_alpha * destination_alpha;
    if output_alpha <= f32::EPSILON {
        return Pixel::TRANSPARENT;
    }

    let source_channels = [source.r, source.g, source.b].map(|value| f32::from(value) / 255.0);
    let destination_channels =
        [destination.r, destination.g, destination.b].map(|value| f32::from(value) / 255.0);
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
        };
        let premultiplied = (1.0 - source_alpha) * destination_value * destination_alpha
            + (1.0 - destination_alpha) * source_value * source_alpha
            + source_alpha * destination_alpha * blended;
        output[channel] = (premultiplied / output_alpha * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
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
