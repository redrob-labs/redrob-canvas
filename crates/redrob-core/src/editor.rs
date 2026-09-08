// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::{HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::filters::apply_filter;
use crate::{Command, CoreError, Document, FrameId, LayerId, RenderSnapshot, Result};

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
    pub changed_layers: Vec<LayerId>,
}

impl ChangeSet {
    fn merge(&mut self, other: &Self) {
        self.document_changed |= other.document_changed;
        self.structure_changed |= other.structure_changed;
        self.canvas_changed |= other.canvas_changed;
        self.timeline_changed |= other.timeline_changed;
        self.navigation_changed |= other.navigation_changed;
        self.selection_changed |= other.selection_changed;
        for id in &other.changed_layers {
            if !self.changed_layers.contains(id) {
                self.changed_layers.push(*id);
            }
        }
        self.generation = self.generation.max(other.generation);
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
            changed_layers: document.layers().iter().map(|layer| layer.id()).collect(),
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

#[derive(Clone)]
struct HistoryEntry {
    before: Document,
    after: Document,
    changes: ChangeSet,
    label: Option<String>,
}

impl HistoryEntry {
    fn new(before: Document, after: Document, changes: ChangeSet, label: Option<String>) -> Self {
        Self {
            before,
            after,
            changes,
            label,
        }
    }

    fn memory_bytes(&self, shared_allocations: &mut HashSet<usize>) -> usize {
        self.before
            .memory_bytes(shared_allocations)
            .saturating_add(self.after.memory_bytes(shared_allocations))
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
            } => {
                let id = document.active_layer_id();
                document.brush_stroke(points, *color, *size, *opacity, *settings)?;
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
        let before = self.document.clone();
        let mut after = before.clone();
        let mut changes = CommandBus::apply(&mut after, &command)?;
        after.stop_playback();
        Self::mark_navigation_changes(&before, &after, &mut changes);
        after.validate()?;
        self.generation = self.generation.saturating_add(1);
        changes.generation = self.generation;
        self.document = after.clone();
        self.history.record(before, after, changes.clone());
        Ok(changes)
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
        let viewed_frame = self.document.current_frame_id();
        let mut after = entry.before.clone();
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
        let viewed_frame = self.document.current_frame_id();
        let mut after = entry.after.clone();
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
        RenderSnapshot::try_render(&self.document, self.generation)
    }

    /// Renders an existing frame without changing current-frame navigation,
    /// generation, or undo/redo history.
    pub fn render_frame_snapshot(&self, frame: crate::FrameId) -> Result<RenderSnapshot> {
        RenderSnapshot::try_render_frame(&self.document, self.generation, frame)
    }

    /// Renders the current hierarchy and returns any semantic/limit error.
    pub fn render_snapshot(&self) -> Result<RenderSnapshot> {
        self.try_render_snapshot()
    }
}
