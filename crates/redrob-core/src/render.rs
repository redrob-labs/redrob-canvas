// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::sync::Arc;

use crate::{
    BlendMode, CoreError, Document, MAX_HIERARCHY_DEPTH, MAX_STORED_RASTER_BYTES, NodeId, NodeKind,
    Pixel, Result,
};

/// Maximum aggregate full-canvas pixel visits performed by one render.
///
/// The budget includes clearing the output, each visible raster composite, and
/// both clearing and compositing every visible isolated group. It permits a
/// largest-supported canvas with one raster nested in one group while bounding
/// amplification from valid wide trees.
pub const MAX_RENDER_PIXEL_VISITS: u64 = 256 * 1024 * 1024;

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
                );
                Ok(())
            }
            NodeKind::Group => {
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
) {
    for (index, (destination, source)) in destination
        .chunks_exact_mut(4)
        .zip(source.chunks_exact(4))
        .enumerate()
    {
        let mut source = Pixel::from_slice(source);
        if let Some(mask) = mask {
            source.a = ((u16::from(source.a) * u16::from(mask[index]) + 127) / 255) as u8;
        }
        composite(Pixel::from_slice(destination), source, opacity, mode).write_to(destination);
    }
}

fn renderer_for(document: &Document, frame: crate::FrameId) -> Renderer<'_> {
    let mut children = HashMap::<Option<NodeId>, Vec<usize>>::new();
    for (index, node) in document.nodes().iter().enumerate() {
        children.entry(node.parent_id()).or_default().push(index);
    }
    Renderer {
        document,
        frame,
        children,
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
    renderer_for(document, document.current_frame_id())
        .preflight_render_work((pixel_bytes / 4) as u64)
}

impl RenderSnapshot {
    pub(crate) fn try_render(document: &Document, generation: u64) -> Result<Self> {
        Self::try_render_frame(document, generation, document.current_frame_id())
    }

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
        let renderer = renderer_for(document, frame);

        let mut output = vec![0_u8; pixel_bytes];
        renderer.render_children(None, &mut output, 0)?;
        Ok(Self {
            width: document.width(),
            height: document.height(),
            generation,
            pixels: output.into(),
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
}
