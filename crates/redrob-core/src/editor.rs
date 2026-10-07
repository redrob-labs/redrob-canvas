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
    /// Set when an indexed document's palette moved a colour this command wrote (J.3-b).
    ///
    /// Reported rather than silent: a user who picks a colour and gets a different one needs to be
    /// told it was the palette, not a broken brush.
    ///
    /// `serde(default)` for the same reason every new Document field carries it: an existing agent
    /// or FFI consumer deserializing a change set it recorded before this field existed must keep
    /// working. The round-trip test caught the omission.
    #[serde(default)]
    pub palette_snapped: bool,
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
        self.palette_snapped |= other.palette_snapped;
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

    pub(crate) fn whole_document(generation: u64, document: &Document) -> Self {
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
            palette_snapped: false,
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

    /// The in-place brush-stroke entry. Its only caller is `execute_brush_stroke`, which has
    /// destructured the command already, so the label is the `brush_stroke` tag written out; a test
    /// pins it to the tag the snapshot path would read from the same command.
    fn patch(patch: PixelPatch, changes: ChangeSet) -> Self {
        Self {
            state: EntryState::Patch(std::sync::Arc::new(patch)),
            changes,
            label: Some("brush_stroke".to_string()),
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

    fn record(
        &mut self,
        before: Document,
        after: Document,
        changes: ChangeSet,
        label: Option<String>,
    ) {
        if let Some(group) = &mut self.group {
            group.after = after;
            group.changes.merge(&changes);
            group.has_commands = true;
        } else {
            self.clear_redo();
            self.push_undo(HistoryEntry::new(before, after, changes, label));
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
    /// M2: layer locks wrap every command. A position-locked node refuses moves and transforms; a
    /// transparency-locked active layer gets its alpha back after the edit, so a paint only
    /// recolours what was already there. (Pixel locks are checked where pixels are written.)
    fn apply(document: &mut Document, command: &Command) -> Result<ChangeSet> {
        let active = document.active_layer_id();
        let position_locked =
            |id: crate::NodeId| document.layer(id).is_some_and(|node| node.locks().position);
        let moves_active = matches!(
            command,
            Command::TransformActive { .. }
                | Command::PerspectiveActive { .. }
                | Command::CageTransform { .. }
                | Command::NPointTransform { .. }
                | Command::PuppetWarp { .. }
                | Command::HandleTransform { .. }
                | Command::FlipActive { .. }
                | Command::RotateActive90 { .. }
        );
        if moves_active && position_locked(active) {
            return Err(CoreError::LayerLocked {
                id: active,
                what: "position",
            });
        }
        // M11: a move or transform of a linked layer moves its linked layers with it. Each gets the
        // same command with itself active; the active node is restored after.
        let linked: Vec<crate::NodeId> = if matches!(
            command,
            Command::TransformActive { .. }
                | Command::FlipActive { .. }
                | Command::RotateActive90 { .. }
        ) {
            document
                .linked_with(active)
                .into_iter()
                .filter(|id| {
                    document
                        .layer(*id)
                        .is_some_and(|n| n.kind() == crate::NodeKind::Raster)
                })
                .collect()
        } else {
            Vec::new()
        };
        if let Some(id) = linked.iter().copied().find(|id| position_locked(*id)) {
            return Err(CoreError::LayerLocked {
                id,
                what: "position",
            });
        }
        if !linked.is_empty() {
            let mut changes = Self::apply_unlocked(document, command)?;
            for id in &linked {
                document.set_active_layer(*id)?;
                let more = Self::apply_unlocked(document, command);
                document.set_active_layer(active)?;
                let more = more?;
                changes.canvas_changed |= more.canvas_changed;
                changes.changed_layers.extend(more.changed_layers);
            }
            return Ok(changes);
        }
        if let Command::AlignLayers { ids, .. } = command
            && let Some(id) = ids.iter().copied().find(|id| position_locked(*id))
        {
            return Err(CoreError::LayerLocked {
                id,
                what: "position",
            });
        }
        let alpha = document.locked_alpha_snapshot();
        let changes = Self::apply_unlocked(document, command)?;
        if let Some((id, frame, alpha)) = alpha {
            document.restore_locked_alpha(id, frame, &alpha);
        }
        Ok(changes)
    }

    fn apply_unlocked(document: &mut Document, command: &Command) -> Result<ChangeSet> {
        // L12: these edit (or read) the active cel as 8-bit RGBA. On a 16/32-bit document they run
        // on an 8-bit copy and only the pixels they change lose depth; before, they read and
        // wrote the deep bytes as if they were 8-bit and garbled the layer.
        let eight_bit_only = matches!(
            command,
            Command::BrushStroke { .. }
                | Command::Fill { .. }
                | Command::FloodFill { .. }
                | Command::GradientFill { .. }
                | Command::Clear
                | Command::ClearOutsideSelection
                | Command::StrokeSelection { .. }
                | Command::EncloseAndFill { .. }
                | Command::SmartPatch { .. }
                | Command::ContentAwareFill
                | Command::Lazybrush { .. }
                | Command::SeamlessClone { .. }
                | Command::TransformActive { .. }
                | Command::PerspectiveActive { .. }
                | Command::CageTransform { .. }
                | Command::WarpBrush { .. }
                | Command::NPointTransform { .. }
                | Command::PuppetWarp { .. }
                | Command::HandleTransform { .. }
                | Command::Transform3d { .. }
                | Command::FlipActive { .. }
                | Command::RotateActive90 { .. }
                | Command::SelectByColor { .. }
                | Command::SelectColorRange { .. }
                | Command::SelectScissors { .. }
                | Command::SelectForeground { .. }
                | Command::PaintSelect { .. }
        );
        if eight_bit_only && let Some(edit) = document.begin_8bit_edit()? {
            let result = Self::apply_unlocked_8bit(document, command);
            document.end_8bit_edit(edit, result.is_ok());
            return result;
        }
        Self::apply_unlocked_8bit(document, command)
    }

    fn apply_unlocked_8bit(document: &mut Document, command: &Command) -> Result<ChangeSet> {
        // U4: a non-affine edit of a smart object (and any transform after one) re-renders it
        // from its source, so warps stay editable-quality like Photoshop's smart objects. A plain
        // affine transform with no warps before it keeps the cheaper composed path below.
        let active = document.active_layer_id();
        let smart_warp = matches!(
            command,
            Command::PerspectiveActive { .. }
                | Command::CageTransform { .. }
                | Command::WarpBrush { .. }
                | Command::NPointTransform { .. }
                | Command::PuppetWarp { .. }
                | Command::HandleTransform { .. }
                | Command::Transform3d { .. }
                | Command::FlipActive { .. }
                | Command::RotateActive90 { .. }
        ) || (matches!(command, Command::TransformActive { .. })
            && document.active_smart_has_warps());
        if smart_warp
            && document
                .layer(active)
                .is_some_and(crate::Layer::is_smart_object)
        {
            document.rerender_smart_object(crate::document::SmartEdit::Warp(command), &|doc, op| {
                Self::apply_unlocked_8bit(doc, op).map(|_| ())
            })?;
            return Ok(ChangeSet {
                document_changed: true,
                canvas_changed: true,
                changed_layers: vec![active],
                ..ChangeSet::default()
            });
        }
        // U5: a filter on a smart object becomes a smart filter; the list can be edited later.
        let smart_filters = match command {
            Command::ApplyFilter { filter }
                if document
                    .layer(active)
                    .is_some_and(crate::Layer::is_smart_object) =>
            {
                Some(crate::document::SmartEdit::AddFilter(filter))
            }
            Command::SetSmartFilters { filters } => {
                if !document
                    .layer(active)
                    .is_some_and(crate::Layer::is_smart_object)
                {
                    return Err(CoreError::InvalidTransform);
                }
                Some(crate::document::SmartEdit::SetFilters(filters.clone()))
            }
            _ => None,
        };
        if let Some(edit) = smart_filters {
            document.rerender_smart_object(edit, &|doc, op| {
                Self::apply_unlocked_8bit(doc, op).map(|_| ())
            })?;
            return Ok(ChangeSet {
                document_changed: true,
                canvas_changed: true,
                changed_layers: vec![active],
                ..ChangeSet::default()
            });
        }
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
            Command::AddPath { id, name, commands } => {
                document.add_path(crate::Path {
                    id: *id,
                    name: name.clone(),
                    commands: commands.clone(),
                    visible: true,
                })?;
                changes.structure_changed = true;
            }
            Command::RemovePath { id } => {
                document.remove_path(*id)?;
                changes.structure_changed = true;
            }
            // Guides and sample points contribute no pixels, so these report `structure_changed`
            // and NOT `canvas_changed` — unlike a channel, whose overlay really does repaint the
            // canvas. The one that is not obvious is `SetGuideSettings`: toggling `show_guides`
            // changes what is drawn ON TOP of the canvas, which is the viewport's business and not
            // a new composite.
            Command::AddGuide {
                id,
                orientation,
                position,
                style,
            } => {
                document.add_guide(*id, *orientation, *position, *style)?;
                changes.structure_changed = true;
            }
            Command::MoveGuide { id, position } => {
                document.move_guide(*id, *position)?;
                changes.structure_changed = true;
            }
            Command::RemoveGuide { id } => {
                document.remove_guide(*id)?;
                changes.structure_changed = true;
            }
            Command::AddSamplePoint { id, x, y } => {
                document.add_sample_point(*id, *x, *y)?;
                changes.structure_changed = true;
            }
            Command::MoveSamplePoint { id, x, y } => {
                document.move_sample_point(*id, *x, *y)?;
                changes.structure_changed = true;
            }
            Command::RemoveSamplePoint { id } => {
                document.remove_sample_point(*id)?;
                changes.structure_changed = true;
            }
            Command::SetGuideSettings { settings } => {
                document.set_guide_settings(*settings);
                changes.structure_changed = true;
            }
            Command::RenamePath { id, name } => {
                document.rename_path(*id, name.clone())?;
                changes.structure_changed = true;
            }
            Command::SetPathVisible { id, visible } => {
                document.set_path_visible(*id, *visible)?;
                changes.structure_changed = true;
            }
            Command::PathFromSelection { name, fit } => {
                document.path_from_selection(name.clone(), *fit)?;
                changes.structure_changed = true;
            }
            Command::SelectionFromPath { id, mode } => {
                document.selection_from_path(*id, *mode)?;
                changes.selection_changed = true;
            }
            Command::StrokePath {
                id,
                color,
                size,
                opacity,
                settings,
            } => {
                // Flattened to points and handed to the ordinary brush path, so a stroked path is
                // the same pixels the user would get dragging the brush along it themselves.
                let points: Vec<crate::BrushPoint> = document
                    .path_stroke_points(*id)?
                    .into_iter()
                    .map(|(x, y)| crate::BrushPoint::new(x, y, 1.0))
                    .collect();
                let layer = document.active_layer_id();
                let damaged =
                    document.brush_stroke(&points, *color, *size, *opacity, settings, None, &[])?;
                changes.damage = Some(damaged);
                changes.changed_layers.push(layer);
            }
            Command::ConvertColorMode {
                mode,
                palette,
                dither,
                cmyk_profile,
            } => {
                document.convert_color_mode(
                    *mode,
                    palette.as_ref(),
                    *dither,
                    cmyk_profile.as_deref(),
                )?;
                changes.canvas_changed = true;
                changes.changed_layers.extend(
                    document
                        .layers()
                        .iter()
                        .filter(|layer| layer.kind() == crate::NodeKind::Raster)
                        .map(|layer| layer.id()),
                );
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
                // P11. An adjustment whose filter has no implementation at the new precision would
                // fail every later render; refuse the change instead, naming the filter.
                for node in document.layers() {
                    if let Some(filter) = node.content().adjustment_filter() {
                        crate::filters::check_adjustment_precision(filter, *precision)?;
                    }
                }
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
            Command::AddAdjustmentNode {
                id,
                name,
                parent,
                sibling_index,
                filter,
            } => {
                document.add_adjustment_node(
                    *id,
                    name.clone(),
                    *parent,
                    *sibling_index,
                    filter.clone(),
                )?;
                changes.structure_changed = true;
                changes.canvas_changed = true;
                changes.changed_layers.push(*id);
                if let Some(parent) = parent {
                    changes.changed_layers.push(*parent);
                }
            }
            Command::SetAdjustmentFilter { id, filter } => {
                document.set_adjustment_filter(*id, filter.clone())?;
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
            Command::DuplicateLayer { source, id } => {
                let parent = document.layer(*source).and_then(|node| node.parent_id());
                document.duplicate_node(*source, *id)?;
                changes.structure_changed = true;
                changes.canvas_changed = true;
                changes.changed_layers.push(*id);
                if let Some(parent) = parent {
                    changes.changed_layers.push(parent);
                }
            }
            Command::MergeDown { id } => {
                let parent = document.layer(*id).and_then(|node| node.parent_id());
                let lower = document.merge_down(*id)?;
                changes.structure_changed = true;
                changes.canvas_changed = true;
                changes.changed_layers.push(*id);
                changes.changed_layers.push(lower);
                if let Some(parent) = parent {
                    changes.changed_layers.push(parent);
                }
            }
            Command::MergeVisible { id } | Command::FlattenImage { id, .. } => {
                let background = match command {
                    Command::FlattenImage { background, .. } => Some(*background),
                    _ => None,
                };
                let before: Vec<crate::NodeId> = document.nodes().iter().map(|n| n.id()).collect();
                document.merge_visible(*id, background)?;
                changes.structure_changed = true;
                changes.canvas_changed = true;
                changes.changed_layers.extend(before);
                changes.changed_layers.push(*id);
            }
            Command::PasteLayer {
                id,
                name,
                rect,
                pixels,
            } => {
                let parent = document
                    .layer(document.active_layer_id())
                    .and_then(|n| n.parent_id());
                document.paste_layer(*id, name.clone(), *rect, pixels)?;
                changes.structure_changed = true;
                changes.canvas_changed = true;
                changes.changed_layers.push(*id);
                if let Some(parent) = parent {
                    changes.changed_layers.push(parent);
                }
            }
            Command::StrokeSelection {
                width,
                color,
                location,
            } => {
                let id = document.active_layer_id();
                document.stroke_selection(*width, *color, *location)?;
                changes.canvas_changed = true;
                changes.changed_layers.push(id);
            }
            Command::SetArtboard { id, artboard } => {
                document.set_artboard(*id, *artboard)?;
                changes.canvas_changed = true;
                changes.changed_layers.push(*id);
            }
            Command::SetLayerBlendIf { id, blend_if } => {
                document.set_blend_if(*id, *blend_if)?;
                changes.canvas_changed = true;
                changes.changed_layers.push(*id);
            }
            Command::ConvertToSmartObject { id } => {
                document.convert_to_smart_object(*id)?;
                changes.changed_layers.push(*id);
            }
            Command::RasterizeSmartObject { id } => {
                document.rasterize_smart_object(*id)?;
                changes.changed_layers.push(*id);
            }
            // U5: handled above, before this match, for a smart object; anything else is refused
            // there too.
            Command::SetSmartFilters { .. } => return Err(CoreError::InvalidTransform),
            Command::LinkLayers { ids, link } => {
                document.link_layers(ids, *link)?;
                changes.changed_layers.extend(ids.iter().copied());
            }
            Command::SetLayerLocks { id, locks } => {
                document.set_layer_locks(*id, *locks)?;
                changes.changed_layers.push(*id);
            }
            Command::SetLayerClipped { id, clipped } => {
                document.set_layer_clipped(*id, *clipped)?;
                changes.canvas_changed = true;
                changes.changed_layers.push(*id);
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
            Command::SelectColorRange {
                color,
                fuzziness,
                range,
                mode,
            } => {
                document.select_color_range(*color, *fuzziness, *range, *mode)?;
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
            Command::SeamlessClone {
                src,
                dst_x,
                dst_y,
                max_refine_scale,
            } => {
                let id = document.active_layer_id();
                document.seamless_clone(*src, *dst_x, *dst_y, *max_refine_scale)?;
                changes.changed_layers.push(id);
            }
            // The four caret commands are intercepted in `execute_internal` because the caret lives
            // on the EDITOR, which the bus cannot see. These arms exist only to keep the match
            // exhaustive and should never run: reaching one means the interception was bypassed, so
            // they assert in debug rather than quietly doing nothing, which is how a dropped
            // keystroke would hide.
            Command::SetTextCaret { .. }
            | Command::MoveTextCaret { .. }
            | Command::InsertAtTextCaret { .. }
            | Command::DeleteAtTextCaret { .. } => {
                debug_assert!(
                    false,
                    "caret commands are handled in Editor::execute_internal, not the bus"
                );
                changes.document_changed = false;
            }
            Command::PaintSelect {
                scribbles,
                stroke_width,
                mode,
            } => {
                document.paint_select(scribbles, *stroke_width, *mode)?;
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
            Command::PuppetWarp {
                src_pts,
                dst_pts,
                sampling,
            } => {
                let id = document.active_layer_id();
                document.puppet_warp(src_pts, dst_pts, *sampling)?;
                changes.changed_layers.push(id);
            }
            Command::HandleTransform { src, dst, sampling } => {
                let id = document.active_layer_id();
                document.handle_transform_active(src, dst, *sampling)?;
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
            Command::ContentAwareFill => {
                let id = document.active_layer_id();
                document.content_aware_fill()?;
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
            Command::CropToSelection => {
                let rect = document
                    .selection()
                    .bounds()
                    .ok_or(CoreError::NoSelection)?;
                document.crop_canvas(rect)?;
                changes.canvas_changed = true;
                changes.selection_changed = true;
                changes
                    .changed_layers
                    .extend(document.layers().iter().map(|layer| layer.id()));
            }
            Command::ClearOutsideSelection => {
                if document.selection().bounds().is_none() {
                    return Err(CoreError::NoSelection);
                }
                let id = document.active_layer_id();
                document.invert_selection();
                let cleared = document.clear_active();
                // Restore the selection whether or not the clear succeeded (a locked layer refuses).
                document.invert_selection();
                cleared?;
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
                if document
                    .layer(id)
                    .is_some_and(crate::Layer::is_smart_object)
                {
                    document.transform_smart_object(*transform, *sampling)?;
                } else {
                    document.transform_active(*transform, *sampling)?;
                }
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
    /// The caret in each text node being edited (L.7).
    ///
    /// On the EDITOR and not in the document, which is what upstream does — `text_tool->x_pos` and
    /// the buffer's marks live on the tool. A caret is editing state, so storing it in
    /// `TextContent` would serialise a cursor into every saved project and change the bytes of
    /// files that have no caret in them.
    text_carets: std::collections::BTreeMap<crate::NodeId, crate::TextCaret>,
    /// A stroke being drawn right now (S1). See [`Editor::begin_live_stroke`].
    live_stroke: Option<LiveStroke>,
}

/// The state of a stroke drawn while the pointer is still down (S1, "live" painting).
///
/// Every extension restores the cel from `before` and repaints the WHOLE stroke so far through
/// the same planner the committed stroke uses. That is what makes the live pixels exactly the
/// committed ones: spacing, smoothing, dyna, dynamics and the stroke-level opacity all see the full
/// point list, never a segment. Nothing here touches history or the generation; the finished
/// stroke is committed by [`Editor::end_live_stroke`] as one ordinary `BrushStroke` command.
struct LiveStroke {
    color: crate::Pixel,
    size: f32,
    opacity: f32,
    settings: crate::BrushSettings,
    tip: Option<crate::BrushTip>,
    pipe: Vec<crate::BrushTip>,
    points: Vec<crate::BrushPoint>,
    layer: LayerId,
    frame: FrameId,
    /// The whole active cel as it was when the stroke began.
    before: Vec<u8>,
    /// What the live paint has covered so far, so a repaint can restore exactly that.
    painted: Option<Rect>,
}

/// The cached frame and the region that has changed since it was made.
struct Projection {
    pixels: Option<std::sync::Arc<[u8]>>,
    damage: crate::render::Damage,
}

/// L11: a render taken off the editor (see [`Editor::detach_render`]). `Send`, so it runs on a
/// worker thread.
pub struct RenderJob {
    document: Document,
    generation: u64,
    damage: crate::render::Damage,
    previous: Option<std::sync::Arc<[u8]>>,
}

impl RenderJob {
    /// Renders the copied document. Hand the result to [`Editor::finish_detached_render`].
    pub fn run(self) -> RenderDone {
        let result = RenderSnapshot::try_render_damage(
            &self.document,
            self.generation,
            self.damage,
            self.previous,
        );
        RenderDone {
            damage: self.damage,
            result,
        }
    }
}

/// L11: a finished [`RenderJob`].
pub struct RenderDone {
    damage: crate::render::Damage,
    result: Result<RenderSnapshot>,
}

impl RenderDone {
    pub fn result(&self) -> &Result<RenderSnapshot> {
        &self.result
    }
    pub fn into_result(self) -> Result<RenderSnapshot> {
        self.result
    }
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
            text_carets: std::collections::BTreeMap::new(),
            live_stroke: None,
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
        // S1: any other edit while a live stroke is down would snapshot its unfinished pixels into
        // history. The stroke is abandoned (its cel restored) first; end_live_stroke takes the
        // stroke out before it executes, so its own commit never lands here with one active.
        self.cancel_live_stroke()?;
        // The two caret-only commands never touch the document, so they are handled here rather
        // than in the bus — the same interception the brush fast path above uses. Routing them
        // through `CommandBus::apply` would clone the whole document and push a history entry for
        // moving a cursor, which is not an edit.
        match &command {
            Command::SetTextCaret { id, insert, anchor } => {
                return self.set_text_caret(*id, *insert, *anchor);
            }
            Command::MoveTextCaret {
                id,
                movement,
                count,
                extend,
            } => {
                return self.move_text_caret(*id, *movement, *count, *extend);
            }
            Command::InsertAtTextCaret { id, text } => {
                let (id, text) = (*id, text.clone());
                return self.edit_at_text_caret(id, Some(&text), 0);
            }
            Command::DeleteAtTextCaret { id, direction } => {
                return self.edit_at_text_caret(*id, None, *direction);
            }
            _ => {}
        }
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
        // An indexed document is snapped back onto its palette BEFORE the document is stored and
        // before history records it, so neither the stored pixels nor the redo side of an undo can
        // hold a colour the palette does not have (J.3-b).
        changes.palette_snapped = after.enforce_palette(changes.damage, &changes.changed_layers);
        // L5c: a CMYK document is brought back inside its gamut the same way.
        changes.palette_snapped |=
            after.enforce_cmyk_gamut(changes.damage, &changes.changed_layers);
        after.validate()?;
        self.generation = self.generation.saturating_add(1);
        changes.generation = self.generation;
        self.document = after.clone();
        self.record_damage(changes.damage);
        // The step is named by the command's own serde tag (`apply_filter`, `add_text_node`, ...),
        // read without serialising the payload. See `serde_tag`.
        let label = crate::serde_tag::serde_tag(&command, "type");
        self.history.record(before, after, changes.clone(), label);
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
        // Snapped before the patch's redo side is copied: taking the copy first would record the
        // off-palette pixels and a redo would reintroduce them (J.3-b).
        let palette_snapped = self.document.enforce_palette(Some(rect), &[layer])
            | self.document.enforce_cmyk_gamut(Some(rect), &[layer]);
        let after = self.document.copy_active_region(rect)?;

        let mut changes = ChangeSet {
            document_changed: true,
            damage: Some(rect),
            changed_layers: vec![layer],
            palette_snapped,
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

    /// H7: forget the cached projection so the next render recomposes everything -- after a font
    /// becomes available, text that fell back to the bitmap font now draws with it.
    pub fn invalidate_render(&self) {
        self.record_damage(None);
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

    /// Starts a stroke that paints while the pointer is still down (S1).
    ///
    /// Only for the case the fast brush path already handles -- an existing cel on the active
    /// layer, outside a history group. Anything else returns `LiveStrokeUnavailable` and the caller
    /// keeps the old behaviour of committing the stroke on release.
    #[allow(clippy::too_many_arguments)]
    pub fn begin_live_stroke(
        &mut self,
        color: crate::Pixel,
        size: f32,
        opacity: f32,
        settings: crate::BrushSettings,
        tip: Option<crate::BrushTip>,
        pipe: Vec<crate::BrushTip>,
    ) -> Result<()> {
        if self.live_stroke.is_some() {
            self.cancel_live_stroke()?;
        }
        // M2: a locked layer commits on release instead, where the command bus applies its locks.
        let locked = self
            .document
            .layer(self.document.active_layer_id())
            .is_some_and(|node| !node.locks().is_empty());
        if self.history.group.is_some() || !self.document.active_cel_exists() || locked {
            return Err(CoreError::LiveStrokeUnavailable);
        }
        let full = Rect::new(0, 0, self.document.width(), self.document.height());
        let before = self.document.copy_active_region(full)?;
        self.live_stroke = Some(LiveStroke {
            color,
            size,
            opacity,
            settings,
            tip,
            pipe,
            points: Vec::new(),
            layer: self.document.active_layer_id(),
            frame: self.document.current_frame_id(),
            before,
            painted: None,
        });
        Ok(())
    }

    /// Adds points to the live stroke and repaints it. Returns the region that changed on screen
    /// (old paint and new paint together). History and the generation are not touched.
    pub fn extend_live_stroke(&mut self, points: &[crate::BrushPoint]) -> Result<ChangeSet> {
        let Some(mut live) = self.live_stroke.take() else {
            return Err(CoreError::LiveStrokeUnavailable);
        };
        // The stroke belongs to the cel it started on; a layer or frame switch mid-stroke ends it.
        if live.layer != self.document.active_layer_id()
            || live.frame != self.document.current_frame_id()
        {
            self.live_stroke = Some(live);
            self.cancel_live_stroke()?;
            return Err(CoreError::LiveStrokeUnavailable);
        }
        if live.points.len() + points.len() > crate::command::MAX_BRUSH_POINTS {
            self.live_stroke = Some(live);
            return Err(CoreError::DocumentLimitExceeded("brush points"));
        }
        live.points.extend_from_slice(points);
        let result = self.repaint_live(&mut live);
        self.live_stroke = Some(live);
        result
    }

    fn repaint_live(&mut self, live: &mut LiveStroke) -> Result<ChangeSet> {
        let width = self.document.width();
        if let Some(rect) = live.painted.take() {
            let original = crate::document::copy_region(&live.before, width, rect)?;
            self.document
                .write_region(live.layer, live.frame, rect, &original)?;
        }
        let plan = self.document.plan_brush_stroke(
            &live.points,
            live.color,
            live.size,
            live.opacity,
            &live.settings,
            live.tip.as_ref(),
            &live.pipe,
        )?;
        let painted = self.document.paint_brush_plan(&plan)?;
        let damage = match live.painted {
            Some(old) => crate::render::union_rect(old, painted),
            None => painted,
        };
        live.painted = Some(painted);
        self.record_damage(Some(damage));
        Ok(ChangeSet {
            generation: self.generation,
            canvas_changed: true,
            damage: Some(damage),
            changed_layers: vec![live.layer],
            ..ChangeSet::default()
        })
    }

    /// Finishes the live stroke: puts the cel back as it was and commits the whole stroke as one
    /// ordinary `BrushStroke` command, so history, undo, redo, recorded actions and the generation
    /// see exactly what they always saw. The committed pixels equal the live ones (same planner,
    /// same points).
    pub fn end_live_stroke(&mut self) -> Result<ChangeSet> {
        let Some(live) = self.live_stroke.take() else {
            return Err(CoreError::LiveStrokeUnavailable);
        };
        self.restore_live(&live)?;
        if live.points.is_empty() {
            return Ok(ChangeSet {
                generation: self.generation,
                ..ChangeSet::default()
            });
        }
        self.execute(Command::BrushStroke {
            points: live.points,
            color: live.color,
            size: live.size,
            opacity: live.opacity,
            settings: live.settings,
            tip: live.tip,
            pipe: live.pipe,
        })
    }

    /// Abandons the live stroke and restores the cel. A no-op when none is active.
    pub fn cancel_live_stroke(&mut self) -> Result<ChangeSet> {
        let Some(live) = self.live_stroke.take() else {
            return Ok(ChangeSet {
                generation: self.generation,
                ..ChangeSet::default()
            });
        };
        let damage = live.painted;
        self.restore_live(&live)?;
        Ok(ChangeSet {
            generation: self.generation,
            canvas_changed: damage.is_some(),
            damage,
            changed_layers: vec![live.layer],
            ..ChangeSet::default()
        })
    }

    pub fn has_live_stroke(&self) -> bool {
        self.live_stroke.is_some()
    }

    fn restore_live(&mut self, live: &LiveStroke) -> Result<()> {
        if let Some(rect) = live.painted {
            let original = crate::document::copy_region(&live.before, self.document.width(), rect)?;
            self.document
                .write_region(live.layer, live.frame, rect, &original)?;
            self.record_damage(Some(rect));
        }
        Ok(())
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
        // S1: a live stroke's pixels are not in history; undoing over them would bake them in.
        self.cancel_live_stroke()?;
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
        self.cancel_live_stroke()?;
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

    /// Labels of the redo steps, NEXT to redo first (None where a step has no label).
    pub fn redo_labels(&self) -> Vec<Option<String>> {
        self.history
            .redo
            .iter()
            .rev()
            .map(|e| e.label.clone())
            .collect()
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

    /// L11: starts a render that runs WITHOUT the editor: the document is copied (layers are
    /// shared, so this is cheap) with the outstanding damage and the last projection, and
    /// [`RenderJob::run`] does the work on any thread. A slow canvas -- a blur adjustment layer,
    /// a huge document -- then no longer holds the editor while it renders. Hand the result back
    /// with [`Self::finish_detached_render`].
    pub fn detach_render(&self) -> RenderJob {
        let mut projection = self.projection.borrow_mut();
        let damage = std::mem::replace(&mut projection.damage, crate::render::Damage::Nothing);
        RenderJob {
            document: self.document.clone(),
            generation: self.generation,
            damage,
            previous: projection.pixels.take(),
        }
    }

    /// L11: takes back a [`Self::detach_render`] result. Damage reported while the job ran stays
    /// outstanding, so the next render brings the frame up to date. When another render landed in
    /// between (it rendered everything, at a newer state) the job's frame is dropped as older.
    pub fn finish_detached_render(&self, done: &RenderDone) {
        let mut projection = self.projection.borrow_mut();
        match &done.result {
            Ok(snapshot) if projection.pixels.is_none() => {
                projection.pixels = Some(snapshot.shared_pixels())
            }
            Ok(_) => {}
            Err(_) => projection.damage = projection.damage.union(done.damage),
        }
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

    /// The caret in a text node, if one has been placed (L.7).
    pub fn text_caret(&self, id: crate::NodeId) -> Option<crate::TextCaret> {
        self.text_carets.get(&id).copied()
    }

    /// The text of a text node, or an error when the node is not one.
    fn text_of(&self, id: crate::NodeId) -> Result<crate::TextContent> {
        let layer = self
            .document
            .layer(id)
            .ok_or(CoreError::LayerNotFound(id))?;
        match layer.content() {
            crate::NodeContent::Text { text } => Ok(text.clone()),
            _ => Err(CoreError::LayerNotFound(id)),
        }
    }

    fn set_text_caret(
        &mut self,
        id: crate::NodeId,
        insert: usize,
        anchor: Option<usize>,
    ) -> Result<ChangeSet> {
        let text = self.text_of(id)?;
        let caret = match anchor {
            Some(anchor) => crate::TextCaret::with_selection(&text.text, anchor, insert),
            None => crate::TextCaret::at(&text.text, insert),
        };
        self.text_carets.insert(id, caret);
        // A caret is not a document change, so nothing is marked dirty and no history entry is
        // pushed. `canvas_changed` because the caret is DRAWN on the canvas overlay.
        Ok(ChangeSet {
            canvas_changed: true,
            ..ChangeSet::default()
        })
    }

    fn move_text_caret(
        &mut self,
        id: crate::NodeId,
        movement: crate::CaretMovement,
        count: i32,
        extend: bool,
    ) -> Result<ChangeSet> {
        let text = self.text_of(id)?;
        // An unplaced caret starts at the beginning, which is where a tool that has just entered
        // the node would put it.
        let current = self
            .text_carets
            .get(&id)
            .copied()
            .unwrap_or_else(|| crate::TextCaret::at(&text.text, 0));
        let moved = crate::text_caret::move_caret(&text.text, current, movement, count, extend);
        self.text_carets.insert(id, moved);
        Ok(ChangeSet {
            canvas_changed: true,
            ..ChangeSet::default()
        })
    }

    /// Replace the caret's selection with `inserted`, or insert at the caret (L.7).
    ///
    /// Routed through `SetTextContent` so the edit is recorded in history exactly as any other text
    /// change is — a typed character is an edit and must be undoable, where moving the caret is
    /// not. The caret is then advanced past what was inserted.
    fn edit_at_text_caret(
        &mut self,
        id: crate::NodeId,
        inserted: Option<&str>,
        direction: i32,
    ) -> Result<ChangeSet> {
        let mut content = self.text_of(id)?;
        let current = self
            .text_carets
            .get(&id)
            .copied()
            .unwrap_or_else(|| crate::TextCaret::at(&content.text, 0));
        let (text, caret) = match inserted {
            Some(inserted) => crate::text_caret::insert_at_caret(&content.text, current, inserted),
            None => crate::text_caret::delete_at_caret(&content.text, current, direction),
        };
        content.text = text;
        let changes = self.execute_internal(Command::SetTextContent { id, text: content })?;
        self.text_carets.insert(id, caret);
        Ok(changes)
    }

    /// The colour under each sample point, in list order (L.1).
    ///
    /// Read from the COMPOSITE, not from the active layer. That is the whole point of a sample
    /// point: a user watches it to see what the image looks like there while editing something
    /// else, so a point over a half-opaque layer must report the blend rather than that layer's
    /// own pixel. Reading the active layer would agree with this on a single opaque layer and
    /// disagree on every stack, which is the version that looks like it works.
    ///
    /// `None` for a point outside the canvas, which is legal: upstream validates nothing on add,
    /// so a point can survive a crop and sit off the image.
    pub fn sample_point_colors(&self) -> Result<Vec<(crate::SamplePointId, Option<crate::Pixel>)>> {
        let snapshot = self.try_render_snapshot()?;
        let rgba = snapshot.rgba8();
        let width = snapshot.width();
        let height = snapshot.height();

        Ok(self
            .document()
            .sample_points()
            .iter()
            .map(|point| {
                let inside = point.x() >= 0
                    && point.y() >= 0
                    && (point.x() as u32) < width
                    && (point.y() as u32) < height;
                let color = inside.then(|| {
                    let offset = ((point.y() as usize) * (width as usize) + point.x() as usize) * 4;
                    crate::Pixel {
                        r: rgba[offset],
                        g: rgba[offset + 1],
                        b: rgba[offset + 2],
                        a: rgba[offset + 3],
                    }
                });
                (point.id(), color)
            })
            .collect())
    }
}
