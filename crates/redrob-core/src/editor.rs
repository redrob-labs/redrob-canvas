// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::{HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::filters::apply_filter;
use crate::{Command, CoreError, Document, FrameId, LayerId, Rect, RenderSnapshot, Result};

const HARD_MAX_HISTORY_ENTRIES: usize = 1_024;

/// A concise description of state invalidated by an edit.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChangeSet {
    pub generation: u64,
    pub document_changed: bool,
    pub structure_changed: bool,
    #[serde(default)]
    pub canvas_changed: bool,
    #[serde(default)]
    pub timeline_changed: bool,
    #[serde(default)]
    pub navigation_changed: bool,
    pub selection_changed: bool,
    /// Set when the edit DROPPED sample bits — a precision change to a narrower width (J.1a).
    ///
    /// Narrowing is a legitimate thing to ask for, so it is reported rather than refused. It is
    /// reported HERE, on the result of the edit that did it, because that is the only place a
    /// caller cannot miss it: a log line is not addressed to anyone, and by the next command the
    /// bytes are already gone.
    #[serde(default)]
    pub precision_narrowed: bool,
    pub changed_layers: Vec<LayerId>,
    /// The region this command damaged, when it could say.
    ///
    /// `None` means "the whole canvas", and that is the DEFAULT on purpose: a command that does not report a
    /// region gets a full render, which is slow, where a command that reports too small a region would get a
    /// stale frame, which is wrong. Only the cheap failure is reachable by forgetting.
    ///
    /// `serde(skip)` keeps it out of the wire format entirely, so every existing FFI and agent consumer sees
    /// a byte-identical change set. It is an internal render hint, not part of the contract.
    #[serde(skip)]
    pub(crate) damage: Option<Rect>,
}

impl ChangeSet {
    fn merge(&mut self, other: &Self) {
        self.document_changed |= other.document_changed;
        self.structure_changed |= other.structure_changed;
        self.canvas_changed |= other.canvas_changed;
        self.timeline_changed |= other.timeline_changed;
        self.navigation_changed |= other.navigation_changed;
        self.selection_changed |= other.selection_changed;
        // A group that narrowed anywhere narrowed. Dropping this on merge would hide the loss
        // behind the one wrapper a user is most likely to perform it inside.
        self.precision_narrowed |= other.precision_narrowed;
        for id in &other.changed_layers {
            if !self.changed_layers.contains(id) {
                self.changed_layers.push(*id);
            }
        }
        self.generation = self.generation.max(other.generation);
        // A merged group is damaged wherever either member was, and `None` (the whole canvas) absorbs
        // anything it is merged with -- so a group containing one unreported command renders fully.
        self.damage = match (self.damage, other.damage) {
            (Some(left), Some(right)) => Some(crate::render::union_rect(left, right)),
            _ => None,
        };
    }

    fn whole_document(generation: u64, document: &Document) -> Self {
        Self {
            generation,
            document_changed: true,
            structure_changed: true,
            canvas_changed: true,
            timeline_changed: true,
            navigation_changed: true,
            selection_changed: true,
            // Undo, redo and load restore bytes that already exist; nothing was narrowed by
            // getting here, so this stays false even though everything else is true.
            precision_narrowed: false,
            changed_layers: document.layers().iter().map(|layer| layer.id()).collect(),
            // A whole-document change reports no region, which the renderer reads as the whole canvas. This
            // is the undo/redo and load path, where the document can have changed anywhere.
            damage: None,
        }
    }
}

/// Bounds for snapshot-based undo history.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HistoryConfig {
    pub max_entries: usize,
    pub memory_budget_bytes: usize,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            max_entries: 256,
            memory_budget_bytes: 256 * 1024 * 1024,
        }
    }
}

impl HistoryConfig {
    fn normalized(self) -> Self {
        Self {
            max_entries: self.max_entries.min(HARD_MAX_HISTORY_ENTRIES),
            memory_budget_bytes: self.memory_budget_bytes,
        }
    }
}

/// What an undo entry restores.
#[derive(Clone)]
enum EntryState {
    /// Whole-document snapshots, for any command. Rasters are `Arc`-shared, so these are cheap to
    /// hold -- but a held snapshot shares the current layer's buffer, and the next write to it then
    /// copies the whole layer (`Arc::make_mut`). At 4000x4000 that copy was 33 of a dab's 37 ms.
    Snapshot {
        before: Box<Document>,
        after: Box<Document>,
    },
    /// One rectangle of one cel, for a brush stroke that only rewrote pixels in a cel that already
    /// existed. Holds no document, so the live layer stays uniquely owned and is painted in place.
    Patch(std::sync::Arc<PixelPatch>),
}

struct PixelPatch {
    layer: LayerId,
    frame: FrameId,
    rect: Rect,
    before: Vec<u8>,
    after: Vec<u8>,
}

#[derive(Clone)]
struct HistoryEntry {
    state: EntryState,
    changes: ChangeSet,
    label: Option<String>,
}

impl HistoryEntry {
    fn new(before: Document, after: Document, changes: ChangeSet, label: Option<String>) -> Self {
        Self {
            state: EntryState::Snapshot {
                before: Box::new(before),
                after: Box::new(after),
            },
            changes,
            label,
        }
    }

    fn patch(patch: PixelPatch, changes: ChangeSet) -> Self {
        Self {
            state: EntryState::Patch(std::sync::Arc::new(patch)),
            changes,
            label: None,
        }
    }

    fn memory_bytes(&self, shared_allocations: &mut HashSet<usize>) -> usize {
        let state = match &self.state {
            EntryState::Snapshot { before, after } => before
                .memory_bytes(shared_allocations)
                .saturating_add(after.memory_bytes(shared_allocations)),
            EntryState::Patch(patch) => patch
                .before
                .capacity()
                .saturating_add(patch.after.capacity()),
        };
        state
            .saturating_add(self.label.as_ref().map_or(0, String::capacity))
            .saturating_add(
                self.changes
                    .changed_layers
                    .capacity()
                    .saturating_mul(std::mem::size_of::<LayerId>()),
            )
    }
}

struct GroupBuilder {
    before: Document,
    after: Document,
    changes: ChangeSet,
    label: Option<String>,
    has_commands: bool,
}

struct History {
    config: HistoryConfig,
    undo: VecDeque<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    memory_bytes: usize,
    group: Option<GroupBuilder>,
}

impl History {
    fn new(config: HistoryConfig) -> Self {
        Self {
            config: config.normalized(),
            undo: VecDeque::new(),
            redo: Vec::new(),
            memory_bytes: 0,
            group: None,
        }
    }

    fn recompute_memory_bytes(&mut self) {
        let mut shared_allocations = HashSet::new();
        self.memory_bytes = self
            .undo
            .iter()
            .chain(&self.redo)
            .fold(0_usize, |total, entry| {
                total.saturating_add(entry.memory_bytes(&mut shared_allocations))
            });
    }

    fn clear_redo(&mut self) {
        self.redo.clear();
        self.recompute_memory_bytes();
    }

    fn push_undo(&mut self, entry: HistoryEntry) {
        self.undo.push_back(entry);
        self.recompute_memory_bytes();
        while self.undo.len() > self.config.max_entries
            || self.memory_bytes > self.config.memory_budget_bytes
        {
            let Some(_removed) = self.undo.pop_front() else {
                break;
            };
            self.recompute_memory_bytes();
        }
    }

    fn record(&mut self, before: Document, after: Document, changes: ChangeSet) {
        if let Some(group) = &mut self.group {
            group.after = after;
            group.changes.merge(&changes);
            group.has_commands = true;
        } else {
            self.clear_redo();
            self.push_undo(HistoryEntry::new(before, after, changes, None));
        }
    }

    fn begin_group(&mut self, before: Document, label: Option<String>) -> Result<()> {
        if self.group.is_some() {
            return Err(CoreError::GroupAlreadyActive);
        }
        self.group = Some(GroupBuilder {
            after: before.clone(),
            before,
            changes: ChangeSet::default(),
            label,
            has_commands: false,
        });
        Ok(())
    }

    fn end_group(&mut self) -> Result<()> {
        let group = self.group.take().ok_or(CoreError::NoActiveGroup)?;
        if group.has_commands {
            self.clear_redo();
            self.push_undo(HistoryEntry::new(
                group.before,
                group.after,
                group.changes,
                group.label,
            ));
        }
        Ok(())
    }

    fn cancel_group(&mut self) -> Result<Document> {
        self.group
            .take()
            .map(|group| group.before)
            .ok_or(CoreError::NoActiveGroup)
    }
}

/// Non-history timeline state changes. Navigation still advances generation so
/// frame-dependent render snapshots and proposals become stale.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Navigation {
    SetCurrentFrame { id: FrameId },
    SetPlaying { playing: bool },
    AdvancePlayback,
}

/// Stateless command dispatcher used by [`Editor`].
#[derive(Clone, Copy, Debug, Default)]
pub struct CommandBus;

impl CommandBus {
    fn apply(document: &mut Document, command: &Command) -> Result<ChangeSet> {
        let mut changes = ChangeSet {
            document_changed: true,
            ..ChangeSet::default()
        };
        match command {
            Command::SetMetadata { metadata } => document.set_metadata(metadata.clone()),
            Command::AddChannel {
                id,
                name,
                from_selection,
            } => {
                document.add_channel(*id, name.clone(), *from_selection)?;
                // A channel is drawn as an overlay, so adding a VISIBLE one changes the canvas even
                // though no layer did. Reporting only `structure_changed` would leave the previous
                // frame on screen with the new channel missing from it.
                changes.structure_changed = true;
                changes.canvas_changed = true;
            }
            Command::SetQuickMask { active } => {
                document.set_quick_mask(*active)?;
                // Both directions change the channel list AND what the canvas shows: entering adds
                // a visible overlay and empties the selection, leaving removes the overlay.
                changes.structure_changed = true;
                changes.canvas_changed = true;
                changes.selection_changed = true;
            }
            Command::RemoveChannel { id } => {
                document.remove_channel(*id)?;
                changes.structure_changed = true;
                changes.canvas_changed = true;
            }
            Command::SetChannelVisible { id, visible } => {
                document.set_channel_visible(*id, *visible)?;
                changes.canvas_changed = true;
            }
            Command::SetChannelOpacity { id, opacity } => {
                document.set_channel_opacity(*id, *opacity)?;
                changes.canvas_changed = true;
            }
            Command::SetChannelColor { id, color } => {
                document.set_channel_color(*id, *color)?;
                changes.canvas_changed = true;
            }
            Command::SetChannelShowMasked { id, show_masked } => {
                document.set_channel_show_masked(*id, *show_masked)?;
                changes.canvas_changed = true;
            }
            Command::RenameChannel { id, name } => {
                document.rename_channel(*id, name.clone())?;
                changes.structure_changed = true;
            }
            Command::SetDocumentPrecision { precision } => {
                if document.set_precision(*precision) {
                    changes.precision_narrowed = true;
                }
                // Every cel's bytes were rewritten, so nothing about the old render survives: no
                // damage region is reported, which means the whole canvas.
                changes.canvas_changed = true;
                changes.changed_layers.extend(
                    document
                        .layers()
                        .iter()
                        .filter(|layer| layer.kind() == crate::NodeKind::Raster)
                        .map(|layer| layer.id()),
                );
            }
            Command::AddFrame { id, index } => {
                document.add_frame(*id, *index)?;
                changes.timeline_changed = true;
            }
            Command::DuplicateFrame { source, id, index } => {
                document.duplicate_frame(*source, *id, *index)?;
                changes.timeline_changed = true;
                changes.canvas_changed = true;
                changes.changed_layers.extend(
                    document
                        .layers()
                        .iter()
                        .filter(|layer| layer.has_raster_cel(*id))
                        .map(|layer| layer.id()),
                );
            }
            Command::RemoveFrame { id } => {
                document.remove_frame(*id)?;
                changes.timeline_changed = true;
                changes.canvas_changed = true;
                changes
                    .changed_layers
                    .extend(document.layers().iter().map(|layer| layer.id()));
            }
            Command::MoveFrame { id, new_index } => {
                document.move_frame(*id, *new_index)?;
                changes.timeline_changed = true;
            }
            Command::SetTimelineFps { fps } => {
                document.set_timeline_fps(*fps)?;
                changes.timeline_changed = true;
            }
            Command::SetPlaybackRange { start, end } => {
                document.set_playback_range(*start, *end)?;
                changes.timeline_changed = true;
            }
            Command::SetLooping { looping } => {
                document.set_looping(*looping);
                changes.timeline_changed = true;
            }
            Command::AddLayer { id, name, index } => {
                document.add_layer(*id, name.clone(), *index)?;
                changes.structure_changed = true;
                changes.changed_layers.push(*id);
            }
            Command::AddGroup {
                id,
                name,
                parent,
                sibling_index,
            } => {
                document.add_group(*id, name.clone(), *parent, *sibling_index)?;
                changes.structure_changed = true;
                changes.changed_layers.push(*id);
                if let Some(parent) = parent {
                    changes.changed_layers.push(*parent);
                }
            }
            Command::AddTextNode {
                id,
                name,
                parent,
                sibling_index,
                text,
            } => {
                document.add_text_node(*id, name.clone(), *parent, *sibling_index, text.clone())?;
                changes.structure_changed = true;
                changes.canvas_changed = true;
                changes.changed_layers.push(*id);
                if let Some(parent) = parent {
                    changes.changed_layers.push(*parent);
                }
            }
            Command::SetTextContent { id, text } => {
                document.set_text_content(*id, text.clone())?;
                changes.canvas_changed = true;
                changes.changed_layers.push(*id);
            }
            Command::AddVectorNode {
                id,
                name,
                parent,
                sibling_index,
                vector,
            } => {
                document.add_vector_node(
                    *id,
                    name.clone(),
                    *parent,
                    *sibling_index,
                    vector.clone(),
                )?;
                changes.structure_changed = true;
                changes.canvas_changed = true;
                changes.changed_layers.push(*id);
                if let Some(parent) = parent {
                    changes.changed_layers.push(*parent);
                }
            }
            Command::AddShapeNode {
                id,
                name,
                parent,
                sibling_index,
                shape,
                paint,
            } => {
                // Expanded here rather than in the document, so the document keeps one way
                // to hold a vector node and the geometry stays out of it.
                let path = shape.to_vector_path(paint)?;
                document.add_vector_node(
                    *id,
                    name.clone(),
                    *parent,
                    *sibling_index,
                    crate::VectorContent { paths: vec![path] },
                )?;
                changes.structure_changed = true;
                changes.canvas_changed = true;
                changes.changed_layers.push(*id);
                if let Some(parent) = parent {
                    changes.changed_layers.push(*parent);
                }
            }
            Command::SetVectorContent { id, vector } => {
                document.set_vector_content(*id, vector.clone())?;
                changes.canvas_changed = true;
                changes.changed_layers.push(*id);
            }
            Command::RasterizeSemanticNode { id } => {
                document.rasterize_semantic_node(*id)?;
                changes.canvas_changed = true;
                changes.changed_layers.push(*id);
            }
            Command::MoveNode {
                id,
                parent,
                sibling_index,
            } => {
                let old_parent = document.layer(*id).and_then(|node| node.parent_id());
                document.move_node(*id, *parent, *sibling_index)?;
                changes.structure_changed = true;
                changes.changed_layers.push(*id);
                for affected in [old_parent, *parent].into_iter().flatten() {
                    if !changes.changed_layers.contains(&affected) {
                        changes.changed_layers.push(affected);
                    }
                }
            }
            Command::AddRasterMask { id } => {
                document.add_raster_mask(*id)?;
                changes.changed_layers.push(*id);
            }
            Command::RemoveRasterMask { id } => {
                document.remove_raster_mask(*id)?;
                changes.changed_layers.push(*id);
            }
            Command::SetRasterMaskEnabled { id, enabled } => {
                document.set_raster_mask_enabled(*id, *enabled)?;
                changes.changed_layers.push(*id);
            }
            Command::RasterMaskFromSelection { id } => {
                document.raster_mask_from_selection(*id)?;
                changes.changed_layers.push(*id);
            }
            Command::ReplaceRasterMask { id, rect, pixels } => {
                document.replace_raster_mask(*id, *rect, pixels)?;
                changes.changed_layers.push(*id);
            }
            Command::RemoveLayer { id } => {
                let parent = document.layer(*id).and_then(|node| node.parent_id());
                let was_active = document.active_layer_id() == *id;
                document.remove_layer(*id)?;
                changes.structure_changed = true;
                changes.changed_layers.push(*id);
                if let Some(parent) = parent {
                    changes.changed_layers.push(parent);
                }
                if was_active {
                    changes.changed_layers.push(document.active_layer_id());
                }
            }
            Command::SetActiveLayer { id } => document.set_active_layer(*id)?,
            Command::RenameLayer { id, name } => {
                document.rename_layer(*id, name.clone())?;
                changes.changed_layers.push(*id);
            }
            Command::SetLayerVisibility { id, visible } => {
                document.set_layer_visibility(*id, *visible)?;
                changes.changed_layers.push(*id);
            }
            Command::SetLayerOpacity { id, opacity } => {
                document.set_layer_opacity(*id, *opacity)?;
                changes.changed_layers.push(*id);
            }
            Command::SetLayerBlendMode { id, mode } => {
                document.set_layer_blend_mode(*id, *mode)?;
                changes.changed_layers.push(*id);
            }
            Command::ReorderLayer { id, new_index } => {
                document.reorder_layer(*id, *new_index)?;
                changes.structure_changed = true;
                changes.changed_layers.push(*id);
            }
            Command::SelectRectangle { rect, mode } => {
                document.select_rect(*rect, *mode);
                changes.selection_changed = true;
            }
            Command::SelectEllipse { rect, mode } => {
                document.select_ellipse(*rect, *mode);
                changes.selection_changed = true;
            }
            Command::SelectPolygon { points, mode } => {
                document.select_polygon(points, *mode);
                changes.selection_changed = true;
            }
            Command::SelectByColor {
                x,
                y,
                tolerance,
                contiguous,
                mode,
            } => {
                document.select_by_color(*x, *y, *tolerance, *contiguous, *mode)?;
                changes.selection_changed = true;
            }
            Command::SelectScissors { anchors, mode } => {
                document.select_scissors(anchors, *mode)?;
                changes.selection_changed = true;
            }
            Command::SelectForeground { fg, bg, mode } => {
                document.select_foreground(fg, bg, *mode)?;
                changes.selection_changed = true;
            }
            Command::AlignLayers {
                ids,
                h,
                v,
                to_canvas,
            } => {
                document.align_layers(ids, *h, *v, *to_canvas)?;
                changes.changed_layers.extend(ids.iter().copied());
            }
            Command::PerspectiveActive { corners, sampling } => {
                let id = document.active_layer_id();
                document.perspective_active(*corners, *sampling)?;
                changes.changed_layers.push(id);
            }
            Command::CageTransform {
                src_cage,
                dst_cage,
                sampling,
            } => {
                let id = document.active_layer_id();
                document.cage_transform(src_cage, dst_cage, *sampling)?;
                changes.changed_layers.push(id);
            }
            Command::WarpBrush {
                points,
                mode,
                radius,
                strength,
                sampling,
            } => {
                let id = document.active_layer_id();
                document.warp_brush(points, *mode, *radius, *strength, *sampling)?;
                changes.changed_layers.push(id);
            }
            Command::NPointTransform {
                src_pts,
                dst_pts,
                sampling,
            } => {
                let id = document.active_layer_id();
                document.npoint_transform(src_pts, dst_pts, *sampling)?;
                changes.changed_layers.push(id);
            }
            Command::Transform3d {
                rot_x,
                rot_y,
                rot_z,
                distance,
                sampling,
            } => {
                let id = document.active_layer_id();
                document.transform3d_active(*rot_x, *rot_y, *rot_z, *distance, *sampling)?;
                changes.changed_layers.push(id);
            }
            Command::EncloseAndFill {
                rect,
                color,
                alpha_threshold,
            } => {
                let id = document.active_layer_id();
                document.enclose_and_fill(*rect, *color, *alpha_threshold)?;
                changes.changed_layers.push(id);
            }
            Command::SmartPatch { search_radius } => {
                let id = document.active_layer_id();
                document.smart_patch(*search_radius)?;
                changes.changed_layers.push(id);
            }
            Command::Lazybrush { scribbles } => {
                let id = document.active_layer_id();
                document.lazybrush(scribbles)?;
                changes.changed_layers.push(id);
            }
            Command::SelectAll => {
                document.select_all();
                changes.selection_changed = true;
            }
            Command::InvertSelection => {
                document.invert_selection();
                changes.selection_changed = true;
            }
            Command::FeatherSelection { radius } => {
                document.feather_selection(*radius)?;
                changes.selection_changed = true;
            }
            Command::GrowSelection { radius } => {
                document.grow_selection(*radius)?;
                changes.selection_changed = true;
            }
            Command::ShrinkSelection { radius } => {
                document.shrink_selection(*radius)?;
                changes.selection_changed = true;
            }
            Command::ClearSelection => {
                document.clear_selection();
                changes.selection_changed = true;
            }
            Command::BrushStroke {
                points,
                color,
                size,
                opacity,
                settings,
                tip,
                pipe,
            } => {
                let id = document.active_layer_id();
                let damaged = document.brush_stroke(
                    points,
                    *color,
                    *size,
                    *opacity,
                    settings,
                    tip.as_ref(),
                    pipe,
                )?;
                changes.damage = Some(damaged);
                changes.changed_layers.push(id);
            }
            Command::GradientFill { kind, stops } => {
                let id = document.active_layer_id();
                document.gradient_fill(*kind, stops)?;
                changes.changed_layers.push(id);
            }
            Command::Fill { color } => {
                let id = document.active_layer_id();
                document.fill_active(*color)?;
                changes.changed_layers.push(id);
            }
            Command::FloodFill {
                x,
                y,
                color,
                options,
            } => {
                let id = document.active_layer_id();
                document.flood_fill_active(*x, *y, *color, *options)?;
                changes.changed_layers.push(id);
            }
            Command::Clear => {
                let id = document.active_layer_id();
                document.clear_active()?;
                changes.changed_layers.push(id);
            }
            Command::ApplyFilter { filter } => {
                let id = document.active_layer_id();
                apply_filter(document, filter)?;
                changes.changed_layers.push(id);
            }
            Command::ApplyGraph { graph } => {
                if !graph.is_valid() {
                    return Err(CoreError::InvalidFilterParameter);
                }
                let id = document.active_layer_id();
                for node in &graph.nodes {
                    if !node.enabled {
                        continue;
                    }
                    if node.amount >= 1.0 {
                        apply_filter(document, &node.filter)?;
                    } else if node.amount > 0.0 {
                        // Run the op on a snapshot, then blend its result back by `amount` -- in LINEAR
                        // LIGHT (H.19), not over the display-encoded bytes. Averaging bytes makes a
                        // half-strength effect look heavier than half: half of black and half of white
                        // is a mid grey in light, which sRGB encodes near 188 rather than 128.
                        document.prepare_active_raster_edit()?;
                        let before = document.active_raster_pixels()?.to_vec();
                        apply_filter(document, &node.filter)?;
                        let after = document.active_raster_pixels()?.to_vec();
                        let width = document.width();
                        let height = document.height();
                        let mut scene =
                            crate::scene::SceneBuffer::from_srgb8(width, height, &before);
                        let result = crate::scene::SceneBuffer::from_srgb8(width, height, &after);
                        scene.mix_from(&result, node.amount);
                        document.replace_active_pixels(scene.to_srgb8())?;
                    }
                }
                changes.changed_layers.push(id);
            }
            Command::ApplyLayerStyle { style } => {
                let id = document.active_layer_id();
                let width = document.width();
                let height = document.height();
                document.prepare_active_raster_edit()?;
                let mut pixels = document.active_raster_pixels()?.to_vec();
                crate::layer_style::apply_layer_style(&mut pixels, width, height, style)?;
                document.replace_active_pixels(pixels)?;
                changes.changed_layers.push(id);
            }
            Command::CropCanvas { rect } => {
                document.crop_canvas(*rect)?;
                changes.canvas_changed = true;
                changes.selection_changed = true;
                changes
                    .changed_layers
                    .extend(document.layers().iter().map(|layer| layer.id()));
            }
            Command::ResizeCanvas {
                width,
                height,
                sampling,
            } => {
                document.resize_canvas(*width, *height, *sampling)?;
                changes.canvas_changed = true;
                changes.selection_changed = true;
                changes
                    .changed_layers
                    .extend(document.layers().iter().map(|layer| layer.id()));
            }
            Command::FlipActive {
                horizontal,
                vertical,
            } => {
                let id = document.active_layer_id();
                document.flip_active(*horizontal, *vertical)?;
                changes.canvas_changed = true;
                changes.changed_layers.push(id);
            }
            Command::RotateActive90 { clockwise } => {
                let id = document.active_layer_id();
                document.rotate_active_90(*clockwise)?;
                changes.canvas_changed = true;
                changes.changed_layers.push(id);
            }
            Command::TransformActive {
                transform,
                sampling,
            } => {
                let id = document.active_layer_id();
                document.transform_active(*transform, *sampling)?;
                changes.canvas_changed = true;
                changes.changed_layers.push(id);
            }
        }
        Ok(changes)
    }

    /// Executes a command through the editor's transactional/history path.
    pub fn execute(editor: &mut Editor, command: Command) -> Result<ChangeSet> {
        editor.execute_internal(command)
    }
}

/// Stateful editing facade and the single transactional command path.
pub struct Editor {
    document: Document,
    generation: u64,
    history: History,
    /// The last projection and what has been damaged since it was produced.
    ///
    /// A `RefCell` because `render_snapshot` takes `&self` -- the Qt bridge and the FFI both render from a
    /// shared reference, and changing that signature would reach every caller for no gain. The cell is never
    /// borrowed across a call that could re-enter it.
    projection: std::cell::RefCell<Projection>,
}

/// The cached frame and the region that has changed since it was made.
struct Projection {
    pixels: Option<std::sync::Arc<[u8]>>,
    damage: crate::render::Damage,
}

impl Default for Projection {
    fn default() -> Self {
        Self {
            pixels: None,
            // Nothing has been rendered yet, so everything is outstanding.
            damage: crate::render::Damage::Everything,
        }
    }
}

impl Editor {
    pub fn new(document: Document) -> Result<Self> {
        Self::with_history_config(document, HistoryConfig::default())
    }

    pub fn with_history_config(mut document: Document, config: HistoryConfig) -> Result<Self> {
        document.stop_playback();
        document.validate()?;
        Ok(Self {
            document,
            generation: 0,
            history: History::new(config),
            projection: std::cell::RefCell::default(),
        })
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn execute(&mut self, command: Command) -> Result<ChangeSet> {
        CommandBus::execute(self, command)
    }

    fn execute_internal(&mut self, command: Command) -> Result<ChangeSet> {
        if self.history.group.is_none()
            && let Command::BrushStroke {
                points,
                color,
                size,
                opacity,
                settings,
                tip,
                pipe,
            } = &command
            && self.document.active_cel_exists()
        {
            return self.execute_brush_stroke(
                points,
                *color,
                *size,
                *opacity,
                settings,
                tip.as_ref(),
                pipe,
            );
        }
        let before = self.document.clone();
        let mut after = before.clone();
        let mut changes = CommandBus::apply(&mut after, &command)?;
        after.stop_playback();
        Self::mark_navigation_changes(&before, &after, &mut changes);
        after.validate()?;
        self.generation = self.generation.saturating_add(1);
        changes.generation = self.generation;
        self.document = after.clone();
        self.record_damage(changes.damage);
        self.history.record(before, after, changes.clone());
        Ok(changes)
    }

    /// A brush stroke into a cel that already exists, painted in place with only the damaged
    /// rectangle kept for undo.
    ///
    /// Equivalent to the snapshot path for everything undo can observe: a stroke changes nothing but
    /// that cel's pixels (and stops playback, as every command does), and the region it writes is known
    /// exactly before it writes (`Document::plan_brush_stroke`). Groups keep the snapshot path, since a
    /// group's entry spans several commands.
    // Mirrors `Document::brush_stroke`'s parameter list; see the note there.
    #[allow(clippy::too_many_arguments)]
    fn execute_brush_stroke(
        &mut self,
        points: &[crate::BrushPoint],
        color: crate::Pixel,
        size: f32,
        opacity: f32,
        settings: &crate::BrushSettings,
        tip: Option<&crate::BrushTip>,
        pipe: &[crate::BrushTip],
    ) -> Result<ChangeSet> {
        let plan = self
            .document
            .plan_brush_stroke(points, color, size, opacity, settings, tip, pipe)?;
        let rect = plan.damage;
        let layer = self.document.active_layer_id();
        let frame = self.document.current_frame_id();
        let before = self.document.copy_active_region(rect)?;
        let was_playing = self.document.timeline().playback().playing;

        self.document.paint_brush_plan(&plan)?;
        self.document.stop_playback();
        if let Err(error) = self.document.validate() {
            self.document.write_region(layer, frame, rect, &before)?;
            return Err(error);
        }
        let after = self.document.copy_active_region(rect)?;

        let mut changes = ChangeSet {
            document_changed: true,
            damage: Some(rect),
            changed_layers: vec![layer],
            ..ChangeSet::default()
        };
        if was_playing {
            changes.timeline_changed = true;
            changes.navigation_changed = true;
        }
        self.generation = self.generation.saturating_add(1);
        changes.generation = self.generation;
        self.record_damage(changes.damage);
        self.history.clear_redo();
        self.history.push_undo(HistoryEntry::patch(
            PixelPatch {
                layer,
                frame,
                rect,
                before,
                after,
            },
            changes.clone(),
        ));
        Ok(changes)
    }

    /// Writes one side of a pixel patch back, for undo (`before`) or redo (`after`).
    fn apply_patch(
        &mut self,
        patch: &PixelPatch,
        bytes: &[u8],
        changes: &ChangeSet,
    ) -> Result<ChangeSet> {
        let was_playing = self.document.timeline().playback().playing;
        self.document
            .write_region(patch.layer, patch.frame, patch.rect, bytes)?;
        self.document.stop_playback();
        let mut changes = changes.clone();
        changes.navigation_changed = true;
        changes.canvas_changed = true;
        if was_playing {
            changes.timeline_changed = true;
        }
        self.generation = self.generation.saturating_add(1);
        changes.generation = self.generation;
        // The patch's own rectangle is exactly what changed.
        self.record_damage(Some(patch.rect));
        Ok(changes)
    }

    /// Notes what a mutation damaged, so the next render can bound itself to it.
    ///
    /// `None` means the whole canvas. Every path that changes the document without reporting a region --
    /// undo, redo, navigation, a direct document replacement -- must reach this with `None`, and the test
    /// `every_mutation_path_marks_damage` walks them to check that none was missed.
    fn record_damage(&self, damage: Option<Rect>) {
        let mut projection = self.projection.borrow_mut();
        let reported = match damage {
            None => crate::render::Damage::Everything,
            // A zero-area region means the command touched no pixel. It must NOT become a `Region` at its
            // corner: a union with an empty box at the origin pulls the result back to the origin.
            Some(rect) if rect.width == 0 || rect.height == 0 => crate::render::Damage::Nothing,
            Some(rect) => crate::render::Damage::Region(rect),
        };
        projection.damage = projection.damage.union(reported);
    }

    fn mark_navigation_changes(before: &Document, after: &Document, changes: &mut ChangeSet) {
        let frame_changed = before.current_frame_id() != after.current_frame_id();
        let playback_changed =
            before.timeline().playback().playing != after.timeline().playback().playing;
        if frame_changed {
            changes.canvas_changed = true;
        }
        if frame_changed || playback_changed {
            changes.timeline_changed = true;
            changes.navigation_changed = true;
        }
    }

    pub fn navigate(&mut self, navigation: Navigation) -> Result<ChangeSet> {
        if self.history.group.is_some() {
            return Err(CoreError::GroupInProgress);
        }
        let mut after = self.document.clone();
        let before_frame = after.current_frame_id();
        let canvas_changed = match navigation {
            Navigation::SetCurrentFrame { id } => {
                after.set_current_frame(id)?;
                true
            }
            Navigation::SetPlaying { playing } => {
                after.set_playing(playing);
                after.current_frame_id() != before_frame
            }
            Navigation::AdvancePlayback => {
                after.advance_playback();
                true
            }
        };
        after.validate()?;
        self.generation = self.generation.saturating_add(1);
        self.document = after;
        // Navigation can change the frame, and a different frame's cels are different pixels everywhere.
        self.record_damage(None);
        Ok(ChangeSet {
            generation: self.generation,
            document_changed: true,
            canvas_changed,
            timeline_changed: true,
            navigation_changed: true,
            ..ChangeSet::default()
        })
    }

    pub fn begin_group(&mut self, label: impl Into<String>) -> Result<()> {
        self.history
            .begin_group(self.document.clone(), Some(label.into()))
    }

    pub fn end_group(&mut self) -> Result<()> {
        self.history.end_group()
    }

    /// Aborts an active group and restores its initial document state.
    pub fn cancel_group(&mut self) -> Result<ChangeSet> {
        self.document = self.history.cancel_group()?;
        self.generation = self.generation.saturating_add(1);
        // The document was replaced wholesale by the group's initial state.
        self.record_damage(None);
        Ok(ChangeSet::whole_document(self.generation, &self.document))
    }

    pub fn undo(&mut self) -> Result<ChangeSet> {
        if self.history.group.is_some() {
            return Err(CoreError::GroupInProgress);
        }
        let entry = self
            .history
            .undo
            .back()
            .cloned()
            .ok_or(CoreError::NothingToUndo)?;
        let before = match &entry.state {
            EntryState::Snapshot { before, .. } => before,
            EntryState::Patch(patch) => {
                let changes = self.apply_patch(patch, &patch.before, &entry.changes)?;
                let committed_entry = self
                    .history
                    .undo
                    .pop_back()
                    .expect("validated undo entry must remain available");
                self.history.redo.push(committed_entry);
                return Ok(changes);
            }
        };
        let viewed_frame = self.document.current_frame_id();
        let mut after = Document::clone(before);
        if after.timeline().contains(viewed_frame) {
            after.set_current_frame(viewed_frame)?;
        }
        after.stop_playback();
        let mut changes = entry.changes.clone();
        Self::mark_navigation_changes(&self.document, &after, &mut changes);
        changes.navigation_changed = true;
        changes.canvas_changed = true;
        after.validate()?;

        self.generation = self.generation.saturating_add(1);
        changes.generation = self.generation;
        self.document = after;
        // A restored snapshot can differ anywhere, and the stored change set's own region describes the
        // command that was undone rather than the difference undoing it makes.
        self.record_damage(None);
        let committed_entry = self
            .history
            .undo
            .pop_back()
            .expect("validated undo entry must remain available");
        self.history.redo.push(committed_entry);
        Ok(changes)
    }

    pub fn redo(&mut self) -> Result<ChangeSet> {
        if self.history.group.is_some() {
            return Err(CoreError::GroupInProgress);
        }
        let entry = self
            .history
            .redo
            .last()
            .cloned()
            .ok_or(CoreError::NothingToRedo)?;
        let restored = match &entry.state {
            EntryState::Snapshot { after, .. } => after,
            EntryState::Patch(patch) => {
                let changes = self.apply_patch(patch, &patch.after, &entry.changes)?;
                let committed_entry = self
                    .history
                    .redo
                    .pop()
                    .expect("validated redo entry must remain available");
                self.history.undo.push_back(committed_entry);
                return Ok(changes);
            }
        };
        let viewed_frame = self.document.current_frame_id();
        let mut after = Document::clone(restored);
        if after.timeline().contains(viewed_frame) {
            after.set_current_frame(viewed_frame)?;
        }
        after.stop_playback();
        let mut changes = entry.changes.clone();
        Self::mark_navigation_changes(&self.document, &after, &mut changes);
        changes.navigation_changed = true;
        changes.canvas_changed = true;
        after.validate()?;

        self.generation = self.generation.saturating_add(1);
        changes.generation = self.generation;
        self.document = after;
        // A restored snapshot can differ anywhere, and the stored change set's own region describes the
        // command that was undone rather than the difference undoing it makes.
        self.record_damage(None);
        let committed_entry = self
            .history
            .redo
            .pop()
            .expect("validated redo entry must remain available");
        self.history.undo.push_back(committed_entry);
        Ok(changes)
    }

    pub fn can_undo(&self) -> bool {
        !self.history.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.history.redo.is_empty()
    }

    /// Number of undoable steps on the stack (F.5 history docker).
    pub fn undo_depth(&self) -> usize {
        self.history.undo.len()
    }

    /// Number of redoable steps on the stack.
    pub fn redo_depth(&self) -> usize {
        self.history.redo.len()
    }

    /// Labels of the undo steps, oldest first (None where a step has no label).
    pub fn undo_labels(&self) -> Vec<Option<String>> {
        self.history.undo.iter().map(|e| e.label.clone()).collect()
    }

    pub fn is_group_active(&self) -> bool {
        self.history.group.is_some()
    }

    pub fn is_selection_active(&self) -> bool {
        self.document.is_selection_active()
    }

    /// Returns owned selection mask bytes associated with the current generation.
    pub fn selection_mask_snapshot(&self) -> Vec<u8> {
        self.document.selection_mask_snapshot()
    }

    pub fn try_render_snapshot(&self) -> Result<RenderSnapshot> {
        // The projection is TAKEN, not cloned. Cloning the `Arc` out and leaving it in the cell keeps two
        // owners alive, so `Arc::get_mut` below always fails and the copy-on-write path copies the whole
        // buffer every single frame -- measured at 4000x4000, that was 92 ms of pure memcpy hiding behind a
        // correct-looking result. Taking it leaves this the only owner unless the CALLER still holds a
        // previous snapshot, which is exactly when a copy is genuinely required.
        let (damage, previous) = {
            let mut projection = self.projection.borrow_mut();
            (projection.damage, projection.pixels.take())
        };
        let snapshot =
            RenderSnapshot::try_render_damage(&self.document, self.generation, damage, previous)?;
        let mut projection = self.projection.borrow_mut();
        projection.pixels = Some(snapshot.shared_pixels());
        // The damage is only cleared once a render has actually SUCCEEDED. An error above leaves it
        // outstanding, so the next attempt still recomputes the region rather than trusting a frame that was
        // never produced.
        projection.damage = crate::render::Damage::Nothing;
        Ok(snapshot)
    }

    /// Renders an existing frame without changing current-frame navigation,
    /// generation, or undo/redo history.
    pub fn render_frame_snapshot(&self, frame: crate::FrameId) -> Result<RenderSnapshot> {
        RenderSnapshot::try_render_frame(&self.document, self.generation, frame)
    }

    /// Renders the CURRENT frame with its neighbours ghosted behind it (H.2).
    ///
    /// The onion-skin composite is not cached in the projection: it depends on arguments the
    /// projection knows nothing about (how many neighbours, which tints), and a cached ghost would be
    /// served to a caller that asked for a different depth. The shell asks for it only while the
    /// animator has onion skin switched on, so the cost is paid where it is wanted.
    pub fn render_onion_skin_snapshot(
        &self,
        before: u32,
        after: u32,
        tint_before: crate::Pixel,
        tint_after: crate::Pixel,
        opacity: f32,
    ) -> Result<RenderSnapshot> {
        crate::render_onion_skin(
            &self.document,
            self.generation,
            self.document.current_frame_id(),
            before,
            after,
            tint_before,
            tint_after,
            opacity,
        )
    }

    /// Renders the current hierarchy and returns any semantic/limit error.
    pub fn render_snapshot(&self) -> Result<RenderSnapshot> {
        self.try_render_snapshot()
    }
}
