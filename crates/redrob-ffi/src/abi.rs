// SPDX-License-Identifier: GPL-3.0-or-later

use std::cell::RefCell;
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::slice;
use std::str;
use std::sync::{Arc, Mutex, MutexGuard};

use redrob_agent::{
    ApiKey, AssistantResponse, ChatMessage, ChatRequest, CompletionLimits, Error as AgentError,
    RedrobClient, RedrobConfig, Usage,
};
use redrob_core::{
    AlphaPolicy, Command, Document, Editor, ExportOptions, FileFormat, FormatWarning, FrameId,
    ImportOptions, LossPolicy, Navigation, Pixel, export_document, export_png, import_document,
    import_png, load_project, save_project,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    ProposalContext, ProposalDto, SharedEditor, proposal_from_tool_call, proposal_tool_declarations,
};

pub const REDROB_OK: i32 = 0;
pub const REDROB_ERROR: i32 = 1;
pub const REDROB_PANIC: i32 = 2;
// Adding a new symbol and a new standalone output struct is backward-compatible:
// no existing struct layout or function signature changed, so ABI v2 remains valid.
pub const REDROB_FFI_ABI_VERSION: u32 = 2;

const MAX_API_KEY_BYTES: usize = 16 * 1024;
const MAX_PROMPT_BYTES: usize = 16 * 1024;
const MAX_TOOL_CALLS: usize = 32;
const MAX_PROPOSALS: usize = 32;
const MAX_TOOL_ARGUMENT_BYTES: usize = 256 * 1024;
const MAX_TOOL_FIELD_BYTES: usize = 512;
const MAX_ASSISTANT_TEXT_BYTES: usize = 256 * 1024;
/// Maximum borrowed command JSON accepted before deserialization.
pub const MAX_COMMAND_JSON_BYTES: usize = 1024 * 1024;
/// Maximum borrowed generic format options JSON accepted before deserialization.
pub const MAX_FORMAT_OPTIONS_JSON_BYTES: usize = 64 * 1024;
const FORMAT_OPTIONS_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FileFormatJson {
    Rrg,
    Png,
    Jpeg,
    Webp,
    Ora,
    Svg,
}

impl From<FileFormatJson> for FileFormat {
    fn from(value: FileFormatJson) -> Self {
        match value {
            FileFormatJson::Rrg => Self::Rrg,
            FileFormatJson::Png => Self::Png,
            FileFormatJson::Jpeg => Self::Jpeg,
            FileFormatJson::Webp => Self::WebP,
            FileFormatJson::Ora => Self::Ora,
            FileFormatJson::Svg => Self::Svg,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LossPolicyJson {
    #[default]
    RejectLoss,
    AllowLoss,
}

impl From<LossPolicyJson> for LossPolicy {
    fn from(value: LossPolicyJson) -> Self {
        match value {
            LossPolicyJson::RejectLoss => Self::RejectLoss,
            LossPolicyJson::AllowLoss => Self::AllowLoss,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MatteJson {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

impl From<MatteJson> for Pixel {
    fn from(value: MatteJson) -> Self {
        Self::rgba(value.r, value.g, value.b, value.a)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(tag = "policy", rename_all = "snake_case", deny_unknown_fields)]
enum JpegAlphaJson {
    #[default]
    RejectNonOpaque,
    Flatten {
        matte: MatteJson,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportFileOptionsJson {
    schema_version: u32,
    expected_format: Option<FileFormatJson>,
    #[serde(default)]
    loss_policy: LossPolicyJson,
    #[serde(default = "default_format_input_limit")]
    max_input_bytes: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportFileOptionsJson {
    schema_version: u32,
    format: FileFormatJson,
    frame: Option<u32>,
    #[serde(default)]
    loss_policy: LossPolicyJson,
    #[serde(default = "default_jpeg_quality")]
    jpeg_quality: u8,
    #[serde(default)]
    jpeg_alpha: JpegAlphaJson,
}

const fn default_format_input_limit() -> usize {
    redrob_core::MAX_FORMAT_INPUT_BYTES
}

const fn default_jpeg_quality() -> u8 {
    90
}

fn ffi_completion_limits() -> CompletionLimits {
    CompletionLimits {
        max_assistant_text_bytes: MAX_ASSISTANT_TEXT_BYTES,
        max_tool_calls: MAX_TOOL_CALLS,
        max_tool_argument_bytes: MAX_TOOL_ARGUMENT_BYTES,
        max_total_tool_argument_bytes: MAX_TOOL_ARGUMENT_BYTES,
        max_tool_call_id_bytes: MAX_TOOL_FIELD_BYTES,
        max_tool_name_bytes: MAX_TOOL_FIELD_BYTES,
    }
}

thread_local! {
    static LAST_ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Explicitly owned output bytes. Non-empty buffers must be returned with
/// [`redrob_buffer_free`]; the foreign caller must never free `data` directly.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RedrobBuffer {
    pub data: *mut u8,
    pub len: usize,
}

impl Default for RedrobBuffer {
    fn default() -> Self {
        Self {
            data: ptr::null_mut(),
            len: 0,
        }
    }
}

/// Immutable, tightly packed, straight-alpha RGBA8 render projection.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RedrobRenderSnapshot {
    pub rgba: RedrobBuffer,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub generation: u64,
}

/// Immutable, tightly packed, one-byte-per-pixel selection projection.
///
/// `active == 0` means the editor has no explicit selection and edits affect
/// the whole canvas. The returned mask still contains the core's owned raw
/// mask bytes and must be released with [`redrob_buffer_free`].
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RedrobSelectionMaskSnapshot {
    pub mask: RedrobBuffer,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub generation: u64,
    pub active: u8,
}

/// Opaque editor storage. Its fields are never exposed in the C header.
pub struct RedrobEditor {
    editor: SharedEditor,
}

fn set_last_error(message: impl Into<String>) {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = message.into());
}

fn clear_last_error() {
    LAST_ERROR.with(|slot| slot.borrow_mut().clear());
}

fn ffi_call(operation: impl FnOnce() -> Result<(), String>) -> i32 {
    clear_last_error();
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(())) => REDROB_OK,
        Ok(Err(message)) => {
            set_last_error(message);
            REDROB_ERROR
        }
        Err(_) => {
            set_last_error("internal panic contained at the Redrob C ABI boundary");
            REDROB_PANIC
        }
    }
}

fn bytes_into_buffer(bytes: Vec<u8>) -> RedrobBuffer {
    if bytes.is_empty() {
        return RedrobBuffer::default();
    }
    let mut bytes = bytes.into_boxed_slice();
    let buffer = RedrobBuffer {
        data: bytes.as_mut_ptr(),
        len: bytes.len(),
    };
    std::mem::forget(bytes);
    buffer
}

unsafe fn borrowed_bytes<'a>(data: *const u8, len: usize, name: &str) -> Result<&'a [u8], String> {
    if data.is_null() {
        if len == 0 {
            return Ok(&[]);
        }
        return Err(format!("{name} is null but length is {len}"));
    }
    // SAFETY: The C contract requires a readable span of exactly `len` bytes
    // for the duration of this call; null was rejected above.
    Ok(unsafe { slice::from_raw_parts(data, len) })
}

unsafe fn editor_from_ptr<'a>(editor: *mut RedrobEditor) -> Result<&'a RedrobEditor, String> {
    // SAFETY: A non-null pointer must originate from redrob_editor_create and
    // remain alive for this call, as documented by the header.
    unsafe { editor.as_ref() }.ok_or_else(|| "editor handle is null".into())
}

fn lock_editor(editor: &RedrobEditor) -> MutexGuard<'_, Editor> {
    editor
        .editor
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

unsafe fn reset_buffer<'a>(
    output: *mut RedrobBuffer,
    name: &str,
) -> Result<&'a mut RedrobBuffer, String> {
    let output = unsafe { output.as_mut() }.ok_or_else(|| format!("{name} pointer is null"))?;
    *output = RedrobBuffer::default();
    Ok(output)
}

unsafe fn reset_two_buffers<'a>(
    first: *mut RedrobBuffer,
    first_name: &str,
    second: *mut RedrobBuffer,
    second_name: &str,
) -> Result<(&'a mut RedrobBuffer, &'a mut RedrobBuffer), String> {
    if !first.is_null() && first == second {
        // One writable destination cannot represent two independently owned buffers.
        // Reset it once before rejecting the alias, without creating overlapping references.
        unsafe { *first = RedrobBuffer::default() };
        return Err(format!("{first_name} and {second_name} must be distinct"));
    }
    if !first.is_null() {
        // SAFETY: The C contract requires every non-null output pointer to be writable.
        unsafe { *first = RedrobBuffer::default() };
    }
    if !second.is_null() {
        // SAFETY: The C contract requires every non-null output pointer to be writable.
        unsafe { *second = RedrobBuffer::default() };
    }
    let first = unsafe { first.as_mut() }.ok_or_else(|| format!("{first_name} pointer is null"))?;
    let second =
        unsafe { second.as_mut() }.ok_or_else(|| format!("{second_name} pointer is null"))?;
    Ok((first, second))
}

fn parse_format_options<T: for<'de> Deserialize<'de>>(
    data: *const u8,
    len: usize,
) -> Result<T, String> {
    if len > MAX_FORMAT_OPTIONS_JSON_BYTES {
        return Err(format!(
            "format options JSON exceeds the {MAX_FORMAT_OPTIONS_JSON_BYTES}-byte limit"
        ));
    }
    let bytes = unsafe { borrowed_bytes(data, len, "format options JSON") }?;
    serde_json::from_slice(bytes).map_err(|error| format!("invalid format options JSON: {error}"))
}

fn validate_format_schema(version: u32) -> Result<(), String> {
    if version != FORMAT_OPTIONS_SCHEMA_VERSION {
        return Err(format!(
            "unsupported format options schema version {version}; expected {FORMAT_OPTIONS_SCHEMA_VERSION}"
        ));
    }
    Ok(())
}

const fn format_name(format: FileFormat) -> &'static str {
    match format {
        FileFormat::Rrg => "rrg",
        FileFormat::Png => "png",
        FileFormat::Jpeg => "jpeg",
        FileFormat::WebP => "webp",
        FileFormat::Ora => "ora",
        FileFormat::Svg => "svg",
        _ => "unknown",
    }
}

fn warning_value(warning: &FormatWarning) -> Value {
    match warning {
        FormatWarning::FlattenedHierarchy => json!({"code": "flattened_hierarchy"}),
        FormatWarning::FlattenedAlpha { matte } => json!({
            "code": "flattened_alpha",
            "matte": matte,
        }),
        FormatWarning::RasterizedSemanticNode { node } => json!({
            "code": "rasterized_semantic_node",
            "node": node,
        }),
        FormatWarning::BakedRasterMask { node } => json!({
            "code": "baked_raster_mask",
            "node": node,
        }),
        FormatWarning::OmittedDisabledMask { node } => json!({
            "code": "omitted_disabled_mask",
            "node": node,
        }),
        FormatWarning::OmittedFrames { exported } => json!({
            "code": "omitted_frames",
            "exported_frame": exported,
        }),
        FormatWarning::OmittedSelection => json!({"code": "omitted_selection"}),
        FormatWarning::OmittedMetadata => json!({"code": "omitted_metadata"}),
        FormatWarning::EmbeddedRasterData { node } => json!({
            "code": "embedded_raster_data",
            "node": node,
        }),
        _ => json!({"code": "unknown_warning"}),
    }
}

fn format_result_json(
    operation: &str,
    metadata: &redrob_core::EffectiveFormatMetadata,
    warnings: &[FormatWarning],
) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&json!({
        "schema_version": 1,
        "operation": operation,
        "detected_format": format_name(metadata.format),
        "effective_format": format_name(metadata.format),
        "width": metadata.width,
        "height": metadata.height,
        "frame": metadata.frame,
        "lossless": metadata.lossless,
        "jpeg_quality": metadata.jpeg_quality,
        "warnings": warnings.iter().map(warning_value).collect::<Vec<_>>(),
    }))
    .map_err(|error| error.to_string())
}

fn format_capabilities_json() -> Result<Vec<u8>, String> {
    serde_json::to_vec(&json!({
        "schema_version": 1,
        "abi_version": REDROB_FFI_ABI_VERSION,
        "limits": {
            "options_json_bytes": MAX_FORMAT_OPTIONS_JSON_BYTES,
            "input_bytes": redrob_core::MAX_FORMAT_INPUT_BYTES,
            "output_bytes": redrob_core::MAX_FORMAT_OUTPUT_BYTES,
            "jpeg_quality_min": 1,
            "jpeg_quality_max": 100,
        },
        "formats": [
            {
                "format": "rrg", "extensions": ["rrg"],
                "operations": ["import", "export"], "lossless": true,
                "scope": "editable_project", "limitations": []
            },
            {
                "format": "png", "extensions": ["png"],
                "operations": ["import", "export"], "lossless": true,
                "scope": "flattened_interchange",
                "limitations": ["export_requires_allow_loss_when_document_structure_or_state_is_omitted"]
            },
            {
                "format": "jpeg", "extensions": ["jpg", "jpeg"],
                "operations": ["import", "export"], "lossless": false,
                "scope": "flattened_interchange",
                "limitations": ["export_requires_opaque_pixels_or_explicit_opaque_matte", "quality_1_to_100"]
            },
            {
                "format": "webp", "extensions": ["webp"],
                "operations": ["import", "export"], "lossless": true,
                "scope": "flattened_interchange", "encoder": "lossless_only",
                "limitations": ["export_requires_allow_loss_when_document_structure_or_state_is_omitted"]
            },
            {
                "format": "ora", "extensions": ["ora"],
                "operations": ["import", "export"], "lossless": true,
                "scope": "layered_interchange",
                "limitations": ["limited_redrob_core_subset", "unsupported_features_are_rejected_or_reported_as_explicit_loss"]
            },
            {
                "format": "svg", "extensions": ["svg"],
                "operations": ["import", "export"], "lossless": true,
                "scope": "limited_svg_interchange",
                "limitations": ["restricted_svg_subset", "raster_data_may_be_embedded_with_machine_readable_warning"]
            }
        ],
        "adapters": {
            "gegl": {"compiled": false, "initialized": false, "ready": false, "operations": [], "formats": []},
            "krita": {"compiled": false, "scaffold_compiled": false, "attached": false, "ready": false, "operations": [], "formats": []}
        }
    }))
    .map_err(|error| error.to_string())
}

fn changes_json(changes: &redrob_core::ChangeSet) -> Result<Vec<u8>, String> {
    serde_json::to_vec(changes).map_err(|error| error.to_string())
}

fn vector_rectangle_value(vector: &redrob_core::VectorContent) -> Option<Value> {
    let [path] = vector.paths.as_slice() else {
        return None;
    };
    let [
        redrob_core::PathCommand::MoveTo { x, y },
        redrob_core::PathCommand::LineTo { x: x1, y: y1 },
        redrob_core::PathCommand::LineTo { x: x2, y: y2 },
        redrob_core::PathCommand::LineTo { x: x3, y: y3 },
        redrob_core::PathCommand::Close,
    ] = path.commands.as_slice()
    else {
        return None;
    };
    if path.fill.is_none() || path.stroke.is_none() {
        return None;
    }
    if *y1 != *y || *x2 != *x1 || *y3 != *y2 || *x3 != *x || *x1 <= *x || *y2 <= *y {
        return None;
    }
    Some(json!({
        "x": x,
        "y": y,
        "width": x1 - x,
        "height": y2 - y,
        "fill": path.fill,
        "stroke_color": path.stroke.map(|stroke| stroke.color),
        "stroke_width": path.stroke.map(|stroke| stroke.width),
        "has_fill": path.fill.is_some(),
        "has_stroke": path.stroke.is_some()
    }))
}

fn semantic_node_value(layer: &redrob_core::Layer) -> Value {
    match layer.content() {
        redrob_core::NodeContent::Text { text } => json!({
            "text": text.text,
            "preview": text.text.chars().take(256).collect::<String>(),
            "preview_truncated": text.text.chars().count() > 256,
            "character_count": text.text.chars().count(),
            "font_id": text.font_id,
            "font_family": text.font_family,
            "font_size": text.font_size,
            "origin_x": text.origin_x,
            "origin_y": text.origin_y,
            "color": text.color
        }),
        redrob_core::NodeContent::Vector { vector } => {
            let rectangle = vector_rectangle_value(vector);
            json!({
                "path_count": vector.paths.len(),
                "command_count": vector.paths.iter().map(|path| path.commands.len()).sum::<usize>(),
                "filled_path_count": vector.paths.iter().filter(|path| path.fill.is_some()).count(),
                "stroked_path_count": vector.paths.iter().filter(|path| path.stroke.is_some()).count(),
                "rectangle_recognized": rectangle.is_some(),
                "rectangle": rectangle
            })
        }
        _ => Value::Null,
    }
}

fn node_value(document: &Document, index: usize, layer: &redrob_core::Layer) -> Value {
    let kind = layer.kind();
    let child_count = document.child_ids(Some(layer.id())).len();
    let semantic = semantic_node_value(layer);
    let can_edit_vector = semantic
        .get("rectangle_recognized")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    json!({
        "index": index,
        "id": layer.id(),
        "name": layer.name(),
        "visible": layer.is_visible(),
        "opacity": layer.opacity(),
        "blend_mode": layer.blend_mode(),
        "kind": kind,
        "parent_id": layer.parent_id(),
        "depth": document.node_depth(layer.id()).unwrap_or(0),
        "has_mask": layer.mask().is_some(),
        "mask_enabled": layer.mask().is_some_and(|mask| mask.is_enabled()),
        "semantic": semantic,
        "group": (kind == redrob_core::NodeKind::Group).then(|| json!({
            "child_count": child_count
        })),
        "capabilities": {
            "can_have_children": kind == redrob_core::NodeKind::Group,
            "can_have_mask": matches!(kind, redrob_core::NodeKind::Raster | redrob_core::NodeKind::Group),
            "can_edit_raster": kind == redrob_core::NodeKind::Raster,
            "can_edit_text": kind == redrob_core::NodeKind::Text,
            "can_edit_vector": can_edit_vector,
            "can_rasterize": matches!(kind, redrob_core::NodeKind::Text | redrob_core::NodeKind::Vector),
            "is_semantic": matches!(kind, redrob_core::NodeKind::Text | redrob_core::NodeKind::Vector),
            "supports_pass_through": false
        }
    })
}

fn layers_value(editor: &Editor) -> Value {
    let document = editor.document();
    json!({
        "schema_version": 2,
        "active_layer_id": document.active_layer_id(),
        "active_node_id": document.active_layer_id(),
        "generation": editor.generation(),
        "layers": document.layers().iter().enumerate().map(|(index, layer)| {
            node_value(document, index, layer)
        }).collect::<Vec<_>>()
    })
}

fn agent_semantic_node_value(layer: &redrob_core::Layer) -> Value {
    match layer.content() {
        redrob_core::NodeContent::Text { text } => json!({
            "character_count": text.text.chars().count(),
            "byte_count": text.text.len(),
            "line_count": text.text.split('\n').count(),
            "font_id": text.font_id,
            "font_family": text.font_family,
            "font_size": text.font_size,
            "origin_x": text.origin_x,
            "origin_y": text.origin_y,
            "color": text.color
        }),
        redrob_core::NodeContent::Vector { vector } => json!({
            "path_count": vector.paths.len(),
            "command_count": vector.paths.iter().map(|path| path.commands.len()).sum::<usize>(),
            "filled_path_count": vector.paths.iter().filter(|path| path.fill.is_some()).count(),
            "stroked_path_count": vector.paths.iter().filter(|path| path.stroke.is_some()).count()
        }),
        _ => Value::Null,
    }
}

fn agent_layers_value(editor: &Editor) -> Value {
    let document = editor.document();
    json!({
        "schema_version": 2,
        "active_node_id": document.active_layer_id(),
        "generation": editor.generation(),
        "layers": document.layers().iter().enumerate().map(|(index, layer)| json!({
            "index": index,
            "id": layer.id(),
            "name": layer.name(),
            "visible": layer.is_visible(),
            "opacity": layer.opacity(),
            "blend_mode": layer.blend_mode(),
            "kind": layer.kind(),
            "parent_id": layer.parent_id(),
            "depth": document.node_depth(layer.id()).unwrap_or(0),
            "semantic": agent_semantic_node_value(layer),
            "capabilities": {
                "can_edit_raster": layer.kind() == redrob_core::NodeKind::Raster,
                "can_edit_text": layer.kind() == redrob_core::NodeKind::Text,
                "can_edit_vector": layer.kind() == redrob_core::NodeKind::Vector,
                "can_rasterize": matches!(layer.kind(), redrob_core::NodeKind::Text | redrob_core::NodeKind::Vector)
            }
        })).collect::<Vec<_>>()
    })
}

fn document_value(editor: &Editor) -> Value {
    let document = editor.document();
    json!({
        "schema_version": 2,
        "id": document.id(),
        "width": document.width(),
        "height": document.height(),
        "generation": editor.generation(),
        "metadata": document.metadata(),
        "can_undo": editor.can_undo(),
        "can_redo": editor.can_redo(),
        "active_layer_id": document.active_layer_id(),
        "active_node_id": document.active_layer_id(),
        "layer_count": document.layers().len(),
        "node_count": document.nodes().len(),
        "timeline": timeline_value(editor)
    })
}

fn timeline_value(editor: &Editor) -> Value {
    let timeline = editor.document().timeline();
    let start = timeline
        .frame_index(timeline.playback().range_start)
        .unwrap_or(0);
    let end = timeline
        .frame_index(timeline.playback().range_end)
        .unwrap_or(start);
    json!({
        "schema_version": 2,
        "generation": editor.generation(),
        "frames": timeline.frames().iter().enumerate().map(|(index, frame)| json!({
            "id": frame.id(),
            "index": index,
            "duration_ms": frame.duration_ms(),
            "current": frame.id() == timeline.current_frame(),
            "in_range": (start..=end).contains(&index)
        })).collect::<Vec<_>>(),
        "current_frame": timeline.current_frame(),
        "fps": timeline.fps(),
        "range_start": timeline.playback().range_start,
        "range_end": timeline.playback().range_end,
        "looping": timeline.playback().looping,
        "playing": timeline.playback().playing
    })
}

fn state_value(editor: &Editor) -> Value {
    json!({
        "schema_version": 2,
        "generation": editor.generation(),
        "document": document_value(editor),
        "layers": layers_value(editor),
        "timeline": timeline_value(editor)
    })
}

#[derive(Debug, Serialize)]
struct AgentProposalResponse {
    assistant_text: String,
    usage: Option<Usage>,
    proposals: Vec<ProposalDto>,
}

fn safe_agent_error(error: &AgentError) -> String {
    match error {
        AgentError::MissingApiKey => "a Redrob API key is required".into(),
        AgentError::InvalidConfiguration(_) => "Redrob agent configuration is invalid".into(),
        AgentError::Http(_) => "Redrob request could not be completed".into(),
        AgentError::HttpStatus { status, .. } => {
            format!("Redrob request failed with HTTP {}", status.as_u16())
        }
        AgentError::Decode(_)
        | AgentError::Sse(_)
        | AgentError::Protocol(_)
        | AgentError::CompletionLimitExceeded { .. } => {
            "Redrob returned an invalid streaming response".into()
        }
        _ => "Redrob agent request failed safely".into(),
    }
}

fn proposal_response(
    response: AssistantResponse,
    context: &ProposalContext,
) -> Result<AgentProposalResponse, String> {
    if response.tool_calls.len() > MAX_TOOL_CALLS {
        return Err(format!(
            "Redrob returned too many tool calls (maximum {MAX_TOOL_CALLS})"
        ));
    }
    let assistant_text = response.content.unwrap_or_default();
    if assistant_text.len() > MAX_ASSISTANT_TEXT_BYTES {
        return Err("Redrob assistant response is too large".into());
    }

    let mut argument_bytes = 0_usize;
    let mut call_ids = HashSet::new();
    let mut proposals = Vec::with_capacity(response.tool_calls.len());
    for call in response.tool_calls {
        if call.id.is_empty()
            || call.id.len() > MAX_TOOL_FIELD_BYTES
            || call.name.is_empty()
            || call.name.len() > MAX_TOOL_FIELD_BYTES
            || !call_ids.insert(call.id.clone())
        {
            return Err("Redrob returned invalid or duplicate tool call metadata".into());
        }
        argument_bytes = argument_bytes
            .checked_add(
                serde_json::to_vec(&call.arguments)
                    .map_err(|_| "Redrob returned invalid tool arguments")?
                    .len(),
            )
            .ok_or_else(|| "Redrob tool arguments are too large".to_string())?;
        if argument_bytes > MAX_TOOL_ARGUMENT_BYTES {
            return Err("Redrob tool arguments are too large".into());
        }
        match proposal_from_tool_call(&call, context) {
            Ok(Some(proposal)) => proposals.push(proposal),
            Ok(None) => {}
            Err(_) => return Err("Redrob returned an invalid graphics tool call".into()),
        }
        if proposals.len() > MAX_PROPOSALS {
            return Err(format!(
                "Redrob returned too many proposals (maximum {MAX_PROPOSALS})"
            ));
        }
    }

    Ok(AgentProposalResponse {
        assistant_text,
        usage: response.usage,
        proposals,
    })
}

fn agent_system_prompt(document: &Value, layers: &Value) -> Result<String, String> {
    let context = serde_json::to_string(&json!({
        "document": document,
        "layers": layers
    }))
    .map_err(|_| "could not serialize editor context".to_string())?;
    Ok(format!(
        "You are the Redrob Canvas proposal assistant. The following editor context JSON is the \
         authoritative immutable snapshot for this request: {context}\n\
         Use only the provided proposal tools, and make every tool call describe a reviewable edit. \
         Never claim an edit was applied: calls become review proposals and execute only after the \
         user selects Apply. Do not request, repeat, or reveal credentials. Keep assistant text concise."
    ))
}

fn run_agent_proposal(
    api_key: &str,
    prompt: &str,
    document: Value,
    layers: Value,
    context: ProposalContext,
) -> Result<Vec<u8>, String> {
    let key = ApiKey::new(api_key.to_owned()).map_err(|error| safe_agent_error(&error))?;
    let client =
        RedrobClient::new(RedrobConfig::new(key)).map_err(|error| safe_agent_error(&error))?;
    let request = ChatRequest::new(vec![
        ChatMessage::system(agent_system_prompt(&document, &layers)?),
        ChatMessage::user(prompt),
    ])
    .with_tools(proposal_tool_declarations());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "could not initialize the Redrob network runtime".to_string())?;
    let response = runtime
        .block_on(client.complete_chat_with_limits(request, ffi_completion_limits()))
        .map_err(|error| safe_agent_error(&error))?;
    serde_json::to_vec(&proposal_response(response, &context)?)
        .map_err(|_| "could not serialize Redrob proposals".into())
}

#[unsafe(no_mangle)]
pub extern "C" fn redrob_ffi_abi_version() -> u32 {
    REDROB_FFI_ABI_VERSION
}

/// Copies this thread's last ABI error into caller-owned storage.
///
/// # Safety
/// `destination` must be null or writable for `capacity` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_last_error_copy(
    destination: *mut std::ffi::c_char,
    capacity: usize,
) -> usize {
    LAST_ERROR.with(|slot| {
        let message = slot.borrow();
        let bytes = message.as_bytes();
        if !destination.is_null() && capacity > 0 {
            let copied = bytes.len().min(capacity - 1);
            // SAFETY: The caller promises a writable `capacity` byte span.
            unsafe {
                ptr::copy_nonoverlapping(bytes.as_ptr(), destination.cast::<u8>(), copied);
                *destination.add(copied) = 0;
            }
        }
        bytes.len()
    })
}

/// Returns a Rust-owned output allocation to Rust.
///
/// # Safety
/// A non-empty `buffer` must be an unchanged value returned by this ABI and
/// must not have been freed previously.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_buffer_free(buffer: RedrobBuffer) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !buffer.data.is_null() {
            // SAFETY: Non-empty output buffers originate from Box<[u8]> in
            // bytes_into_buffer and must be returned exactly once unchanged.
            let raw = ptr::slice_from_raw_parts_mut(buffer.data, buffer.len);
            unsafe { drop(Box::from_raw(raw)) };
        }
    }));
}

/// Creates a new opaque editor.
///
/// # Safety
/// `out_editor` must be non-null and writable for one pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_create(
    width: u32,
    height: u32,
    out_editor: *mut *mut RedrobEditor,
) -> i32 {
    ffi_call(|| {
        let output = unsafe { out_editor.as_mut() }
            .ok_or_else(|| "out_editor pointer is null".to_string())?;
        *output = ptr::null_mut();
        let document = Document::new(width, height).map_err(|error| error.to_string())?;
        let editor = Editor::new(document).map_err(|error| error.to_string())?;
        *output = Box::into_raw(Box::new(RedrobEditor {
            editor: Arc::new(Mutex::new(editor)),
        }));
        Ok(())
    })
}

/// Destroys an opaque editor.
///
/// # Safety
/// `editor` must be null or a live handle returned by `redrob_editor_create`,
/// and a live handle must be passed exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_destroy(editor: *mut RedrobEditor) {
    if editor.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The pointer must originate from redrob_editor_create and is
        // consumed exactly once by this function.
        unsafe { drop(Box::from_raw(editor)) };
    }));
}

/// Requests inert, reviewable proposals from the hosted Redrob agent.
///
/// The API key and prompt are borrowed UTF-8 spans. The editor is only read
/// while its document/layer context is snapshotted; no command, undo, or redo
/// is executed. On success, `out_json` owns UTF-8 JSON and must be freed with
/// [`redrob_buffer_free`]. This synchronous network call is intended for a
/// native worker thread.
///
/// # Safety
/// `editor` must remain live for the call, both input spans must be readable,
/// and `out_json` must be writable for one [`RedrobBuffer`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_agent_propose(
    editor: *mut RedrobEditor,
    api_key_data: *const u8,
    api_key_len: usize,
    prompt_data: *const u8,
    prompt_len: usize,
    out_json: *mut RedrobBuffer,
) -> i32 {
    ffi_call(|| {
        let output = unsafe { out_json.as_mut() }
            .ok_or_else(|| "output buffer pointer is null".to_string())?;
        *output = RedrobBuffer::default();
        let handle = unsafe { editor_from_ptr(editor) }?;
        if api_key_len > MAX_API_KEY_BYTES {
            return Err("API key exceeds the supported length".into());
        }
        if prompt_len > MAX_PROMPT_BYTES {
            return Err(format!("prompt exceeds the {MAX_PROMPT_BYTES}-byte limit"));
        }
        let api_key_bytes = unsafe { borrowed_bytes(api_key_data, api_key_len, "API key") }?;
        let prompt_bytes = unsafe { borrowed_bytes(prompt_data, prompt_len, "prompt") }?;
        let api_key =
            str::from_utf8(api_key_bytes).map_err(|_| "API key is not valid UTF-8".to_string())?;
        let prompt =
            str::from_utf8(prompt_bytes).map_err(|_| "prompt is not valid UTF-8".to_string())?;
        if api_key.trim().is_empty() {
            return Err("API key must not be empty".into());
        }
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return Err("prompt must not be empty".into());
        }

        let (document, layers, context) = {
            let editor = lock_editor(handle);
            (
                document_value(&editor),
                agent_layers_value(&editor),
                ProposalContext::from_editor(&editor),
            )
        };
        *output = bytes_into_buffer(run_agent_proposal(
            api_key, prompt, document, layers, context,
        )?);
        Ok(())
    })
}

/// Executes a typed command encoded as UTF-8 JSON.
///
/// Success stops playback in the detached command candidate and commits one
/// generation/history transition. Rejection leaves all editor state intact.
///
/// # Safety
/// `editor` must be live, the JSON span must be readable, and
/// `out_changes_json` must be writable for one `RedrobBuffer`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_execute_json(
    editor: *mut RedrobEditor,
    json_data: *const u8,
    json_len: usize,
    out_changes_json: *mut RedrobBuffer,
) -> i32 {
    ffi_call(|| {
        let output = unsafe { reset_buffer(out_changes_json, "changes output buffer") }?;
        let handle = unsafe { editor_from_ptr(editor) }?;
        if json_len > MAX_COMMAND_JSON_BYTES {
            return Err(format!(
                "command JSON exceeds the {MAX_COMMAND_JSON_BYTES}-byte limit"
            ));
        }
        let json_data = unsafe { borrowed_bytes(json_data, json_len, "command JSON") }?;
        let command: Command = serde_json::from_slice(json_data)
            .map_err(|error| format!("invalid command JSON: {error}"))?;
        let changes = lock_editor(handle)
            .execute(command)
            .map_err(|error| error.to_string())?;
        *output = bytes_into_buffer(changes_json(&changes)?);
        Ok(())
    })
}

/// Undoes one edit, preserving the viewed frame when valid and stopping
/// playback only as part of a successful detached history transition.
///
/// # Safety
/// `editor` must be live and `out_changes_json` writable for one buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_undo(
    editor: *mut RedrobEditor,
    out_changes_json: *mut RedrobBuffer,
) -> i32 {
    ffi_call(|| {
        let output = unsafe { reset_buffer(out_changes_json, "changes output buffer") }?;
        let handle = unsafe { editor_from_ptr(editor) }?;
        let changes = lock_editor(handle)
            .undo()
            .map_err(|error| error.to_string())?;
        *output = bytes_into_buffer(changes_json(&changes)?);
        Ok(())
    })
}

/// Redoes one edit, preserving the viewed frame when valid and stopping
/// playback only as part of a successful detached history transition.
///
/// # Safety
/// `editor` must be live and `out_changes_json` writable for one buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_redo(
    editor: *mut RedrobEditor,
    out_changes_json: *mut RedrobBuffer,
) -> i32 {
    ffi_call(|| {
        let output = unsafe { reset_buffer(out_changes_json, "changes output buffer") }?;
        let handle = unsafe { editor_from_ptr(editor) }?;
        let changes = lock_editor(handle)
            .redo()
            .map_err(|error| error.to_string())?;
        *output = bytes_into_buffer(changes_json(&changes)?);
        Ok(())
    })
}

/// Serializes the document summary as JSON.
///
/// # Safety
/// `editor` must be live and `out_json` writable for one buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_document_json(
    editor: *mut RedrobEditor,
    out_json: *mut RedrobBuffer,
) -> i32 {
    ffi_call(|| {
        let output = unsafe { reset_buffer(out_json, "document output buffer") }?;
        let handle = unsafe { editor_from_ptr(editor) }?;
        let editor = lock_editor(handle);
        let bytes =
            serde_json::to_vec(&document_value(&editor)).map_err(|error| error.to_string())?;
        *output = bytes_into_buffer(bytes);
        Ok(())
    })
}

/// Serializes the layer snapshot as JSON.
///
/// # Safety
/// `editor` must be live and `out_json` writable for one buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_layers_json(
    editor: *mut RedrobEditor,
    out_json: *mut RedrobBuffer,
) -> i32 {
    ffi_call(|| {
        let output = unsafe { reset_buffer(out_json, "layers output buffer") }?;
        let handle = unsafe { editor_from_ptr(editor) }?;
        let editor = lock_editor(handle);
        let bytes =
            serde_json::to_vec(&layers_value(&editor)).map_err(|error| error.to_string())?;
        *output = bytes_into_buffer(bytes);
        Ok(())
    })
}

/// Serializes the generation-coherent timeline projection as owned JSON.
///
/// # Safety
/// `editor` must be live and `out_json` writable for one buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_timeline_json(
    editor: *mut RedrobEditor,
    out_json: *mut RedrobBuffer,
) -> i32 {
    ffi_call(|| {
        let output = unsafe { reset_buffer(out_json, "timeline output buffer") }?;
        let handle = unsafe { editor_from_ptr(editor) }?;
        let editor = lock_editor(handle);
        *output = bytes_into_buffer(
            serde_json::to_vec(&timeline_value(&editor)).map_err(|error| error.to_string())?,
        );
        Ok(())
    })
}

/// Serializes document, layers, and timeline from one editor lock acquisition.
///
/// # Safety
/// `editor` must be live and `out_json` writable for one buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_state_json(
    editor: *mut RedrobEditor,
    out_json: *mut RedrobBuffer,
) -> i32 {
    ffi_call(|| {
        let output = unsafe { reset_buffer(out_json, "state output buffer") }?;
        let handle = unsafe { editor_from_ptr(editor) }?;
        let editor = lock_editor(handle);
        *output = bytes_into_buffer(
            serde_json::to_vec(&state_value(&editor)).map_err(|error| error.to_string())?,
        );
        Ok(())
    })
}

fn navigate_editor(
    editor: *mut RedrobEditor,
    out_changes_json: *mut RedrobBuffer,
    navigation: Navigation,
) -> i32 {
    ffi_call(|| {
        let output = unsafe { reset_buffer(out_changes_json, "navigation output buffer") }?;
        let handle = unsafe { editor_from_ptr(editor) }?;
        let changes = lock_editor(handle)
            .navigate(navigation)
            .map_err(|error| error.to_string())?;
        *output = bytes_into_buffer(changes_json(&changes)?);
        Ok(())
    })
}

/// Selects a current frame without recording history.
///
/// # Safety
/// `editor` must be live and `out_changes_json` writable for one buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_set_current_frame(
    editor: *mut RedrobEditor,
    frame_id: u32,
    out_changes_json: *mut RedrobBuffer,
) -> i32 {
    navigate_editor(
        editor,
        out_changes_json,
        Navigation::SetCurrentFrame {
            id: FrameId::new(frame_id),
        },
    )
}

/// Changes runtime playback state without recording history.
///
/// # Safety
/// `editor` must be live and `out_changes_json` writable for one buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_set_playing(
    editor: *mut RedrobEditor,
    playing: bool,
    out_changes_json: *mut RedrobBuffer,
) -> i32 {
    navigate_editor(editor, out_changes_json, Navigation::SetPlaying { playing })
}

/// Advances one playback tick without recording history.
///
/// # Safety
/// `editor` must be live and `out_changes_json` writable for one buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_advance_playback(
    editor: *mut RedrobEditor,
    out_changes_json: *mut RedrobBuffer,
) -> i32 {
    navigate_editor(editor, out_changes_json, Navigation::AdvancePlayback)
}

/// Renders an immutable RGBA snapshot.
///
/// # Safety
/// `editor` must be live and `out_snapshot` writable for one snapshot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_render_rgba(
    editor: *mut RedrobEditor,
    out_snapshot: *mut RedrobRenderSnapshot,
) -> i32 {
    ffi_call(|| {
        let output = unsafe { out_snapshot.as_mut() }
            .ok_or_else(|| "render snapshot output pointer is null".to_string())?;
        *output = RedrobRenderSnapshot::default();
        let handle = unsafe { editor_from_ptr(editor) }?;
        let snapshot = lock_editor(handle)
            .try_render_snapshot()
            .map_err(|error| error.to_string())?;
        *output = RedrobRenderSnapshot {
            rgba: bytes_into_buffer(snapshot.pixels().to_vec()),
            width: snapshot.width(),
            height: snapshot.height(),
            stride: snapshot
                .width()
                .checked_mul(4)
                .ok_or_else(|| "render stride overflow".to_string())?,
            generation: snapshot.generation(),
        };
        Ok(())
    })
}

/// Snapshots the owned one-byte-per-pixel selection mask.
///
/// This is a backward-compatible ABI v2 extension: it adds a new symbol and a
/// standalone struct but does not alter any existing public layout/signature.
///
/// # Safety
/// `editor` must be live and `out_snapshot` writable for one snapshot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_selection_mask(
    editor: *mut RedrobEditor,
    out_snapshot: *mut RedrobSelectionMaskSnapshot,
) -> i32 {
    ffi_call(|| {
        let output = unsafe { out_snapshot.as_mut() }
            .ok_or_else(|| "selection snapshot output pointer is null".to_string())?;
        *output = RedrobSelectionMaskSnapshot::default();
        let handle = unsafe { editor_from_ptr(editor) }?;
        let editor = lock_editor(handle);
        let width = editor.document().width();
        let height = editor.document().height();
        *output = RedrobSelectionMaskSnapshot {
            mask: bytes_into_buffer(editor.selection_mask_snapshot()),
            width,
            height,
            stride: width,
            generation: editor.generation(),
            active: u8::from(editor.is_selection_active()),
        };
        Ok(())
    })
}

/// Returns truthful format and product-adapter capabilities as owned JSON.
///
/// # Safety
/// `out_json` must be writable for one buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_ffi_capabilities_json(out_json: *mut RedrobBuffer) -> i32 {
    ffi_call(|| {
        let output = unsafe { reset_buffer(out_json, "capabilities output buffer") }?;
        *output = bytes_into_buffer(format_capabilities_json()?);
        Ok(())
    })
}

/// Imports a bounded, strictly detected generic file and atomically replaces the editor.
///
/// # Safety
/// Both borrowed input spans must remain readable for the call and `out_result_json`
/// must be writable for one buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_import_file(
    editor: *mut RedrobEditor,
    bytes: *const u8,
    len: usize,
    options_json: *const u8,
    options_len: usize,
    out_result_json: *mut RedrobBuffer,
) -> i32 {
    ffi_call(|| {
        let result_output =
            unsafe { reset_buffer(out_result_json, "import result output buffer") }?;
        let handle = unsafe { editor_from_ptr(editor) }?;
        let options: ImportFileOptionsJson = parse_format_options(options_json, options_len)?;
        validate_format_schema(options.schema_version)?;
        let bytes = unsafe { borrowed_bytes(bytes, len, "file bytes") }?;
        let mut core_options = ImportOptions::default()
            .with_loss_policy(options.loss_policy.into())
            .with_max_input_bytes(options.max_input_bytes);
        if let Some(expected) = options.expected_format {
            core_options = core_options.with_expected_format(expected.into());
        }
        let outcome = import_document(bytes, &core_options).map_err(|error| error.to_string())?;
        let result_json = format_result_json("import", outcome.metadata(), outcome.warnings())?;
        let replacement =
            Editor::new(outcome.into_document()).map_err(|error| error.to_string())?;
        *lock_editor(handle) = replacement;
        *result_output = bytes_into_buffer(result_json);
        Ok(())
    })
}

/// Exports one bounded generic file from an immutable editor snapshot.
///
/// # Safety
/// The options span must remain readable for the call. Both outputs must be
/// writable for one buffer and are reset before every failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_export_file(
    editor: *mut RedrobEditor,
    options_json: *const u8,
    options_len: usize,
    out_bytes: *mut RedrobBuffer,
    out_result_json: *mut RedrobBuffer,
) -> i32 {
    ffi_call(|| {
        let (bytes_output, result_output) = unsafe {
            reset_two_buffers(
                out_bytes,
                "file output buffer",
                out_result_json,
                "export result output buffer",
            )
        }?;
        let handle = unsafe { editor_from_ptr(editor) }?;
        let options: ExportFileOptionsJson = parse_format_options(options_json, options_len)?;
        validate_format_schema(options.schema_version)?;
        let format: FileFormat = options.format.into();
        let mut core_options = ExportOptions::default()
            .with_loss_policy(options.loss_policy.into())
            .with_jpeg_quality(options.jpeg_quality);
        if let Some(frame) = options.frame {
            core_options = core_options.with_frame(FrameId::new(frame));
        }
        core_options = core_options.with_alpha_policy(match options.jpeg_alpha {
            JpegAlphaJson::RejectNonOpaque => AlphaPolicy::RejectNonOpaque,
            JpegAlphaJson::Flatten { matte } => AlphaPolicy::Flatten {
                matte: matte.into(),
            },
        });

        // Cloning under the mutex gives export an immutable coherent snapshot;
        // all potentially expensive format encoding runs after the lock is released.
        let document = { lock_editor(handle).document().clone() };
        let outcome =
            export_document(&document, format, &core_options).map_err(|error| error.to_string())?;
        let result_json = format_result_json("export", outcome.metadata(), outcome.warnings())?;
        let bytes = outcome.into_bytes();
        *bytes_output = bytes_into_buffer(bytes);
        *result_output = bytes_into_buffer(result_json);
        Ok(())
    })
}

/// Replaces the document from borrowed RRG bytes.
///
/// # Safety
/// `editor` must be live and the input span readable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_load_rrg(
    editor: *mut RedrobEditor,
    bytes: *const u8,
    len: usize,
) -> i32 {
    ffi_call(|| {
        let handle = unsafe { editor_from_ptr(editor) }?;
        let bytes = unsafe { borrowed_bytes(bytes, len, "RRG bytes") }?;
        let document = load_project(bytes).map_err(|error| error.to_string())?;
        *lock_editor(handle) = Editor::new(document).map_err(|error| error.to_string())?;
        Ok(())
    })
}

/// Serializes the document to RRG bytes.
///
/// # Safety
/// `editor` must be live and `out_bytes` writable for one buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_save_rrg(
    editor: *mut RedrobEditor,
    out_bytes: *mut RedrobBuffer,
) -> i32 {
    ffi_call(|| {
        let output = unsafe { reset_buffer(out_bytes, "project output buffer") }?;
        let handle = unsafe { editor_from_ptr(editor) }?;
        let editor = lock_editor(handle);
        let bytes = save_project(editor.document()).map_err(|error| error.to_string())?;
        *output = bytes_into_buffer(bytes);
        Ok(())
    })
}

/// Replaces the document from borrowed PNG bytes.
///
/// # Safety
/// `editor` must be live and the input span readable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_import_png(
    editor: *mut RedrobEditor,
    bytes: *const u8,
    len: usize,
) -> i32 {
    ffi_call(|| {
        let handle = unsafe { editor_from_ptr(editor) }?;
        let bytes = unsafe { borrowed_bytes(bytes, len, "PNG bytes") }?;
        let document = import_png(bytes).map_err(|error| error.to_string())?;
        *lock_editor(handle) = Editor::new(document).map_err(|error| error.to_string())?;
        Ok(())
    })
}

/// Flattens and encodes the document as PNG bytes.
///
/// # Safety
/// `editor` must be live and `out_bytes` writable for one buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn redrob_editor_export_png(
    editor: *mut RedrobEditor,
    out_bytes: *mut RedrobBuffer,
) -> i32 {
    ffi_call(|| {
        let output = unsafe { reset_buffer(out_bytes, "PNG output buffer") }?;
        let handle = unsafe { editor_from_ptr(editor) }?;
        let editor = lock_editor(handle);
        let bytes = export_png(editor.document()).map_err(|error| error.to_string())?;
        *output = bytes_into_buffer(bytes);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use redrob_agent::{AssistantResponse, ToolCall, Usage};
    use redrob_core::{Document, Editor};
    use serde_json::json;

    use super::{
        MAX_ASSISTANT_TEXT_BYTES, MAX_TOOL_ARGUMENT_BYTES, MAX_TOOL_CALLS, MAX_TOOL_FIELD_BYTES,
        ProposalContext, agent_system_prompt, ffi_completion_limits, proposal_response,
    };

    fn response(tool_calls: Vec<ToolCall>) -> AssistantResponse {
        AssistantResponse {
            content: Some("Review the proposed edits.".into()),
            tool_calls,
            usage: Some(Usage {
                prompt_tokens: 3,
                completion_tokens: 4,
                total_tokens: 7,
            }),
            finish_reason: None,
        }
    }

    fn context() -> ProposalContext {
        let editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
        ProposalContext::from_editor(&editor)
    }

    #[test]
    fn accumulated_response_allows_text_without_proposals() {
        let context = context();
        let result = proposal_response(response(Vec::new()), &context).unwrap();
        assert_eq!(result.assistant_text, "Review the proposed edits.");
        assert!(result.proposals.is_empty());
        assert_eq!(result.usage.unwrap().total_tokens, 7);
    }

    #[test]
    fn accumulated_response_excludes_inspection_and_bounds_tool_calls() {
        let context = context();
        let inspect = ToolCall {
            id: "inspect-1".into(),
            name: "inspect_document".into(),
            arguments: json!({}),
        };
        assert!(
            proposal_response(response(vec![inspect]), &context)
                .unwrap()
                .proposals
                .is_empty()
        );

        let excessive = (0..=MAX_TOOL_CALLS)
            .map(|index| ToolCall {
                id: format!("call-{index}"),
                name: "undo".into(),
                arguments: json!({}),
            })
            .collect();
        assert!(proposal_response(response(excessive), &context).is_err());
    }

    #[test]
    fn accumulated_response_rejects_duplicate_calls_and_malformed_arguments() {
        let context = context();
        let duplicate = ToolCall {
            id: "same".into(),
            name: "fill".into(),
            arguments: json!({ "color": { "r": 1, "g": 2, "b": 3, "a": 255 } }),
        };
        assert!(proposal_response(response(vec![duplicate.clone(), duplicate]), &context).is_err());
        assert!(
            proposal_response(
                response(vec![ToolCall {
                    id: "bad-fill".into(),
                    name: "fill".into(),
                    arguments: json!({ "color": "red" }),
                }]),
                &context,
            )
            .is_err()
        );
    }

    #[test]
    fn one_shot_prompt_uses_embedded_context_without_inspection_dead_end() {
        let prompt =
            agent_system_prompt(&json!({ "generation": 4 }), &json!({ "layers": [] })).unwrap();
        assert!(prompt.contains("authoritative immutable snapshot"));
        assert!(prompt.contains("reviewable edit"));
        assert!(!prompt.contains("inspect_document"));
    }

    #[test]
    fn ffi_completion_limits_match_existing_policy_constants() {
        let limits = ffi_completion_limits();
        assert_eq!(limits.max_assistant_text_bytes, MAX_ASSISTANT_TEXT_BYTES);
        assert_eq!(limits.max_tool_calls, MAX_TOOL_CALLS);
        assert_eq!(limits.max_tool_argument_bytes, MAX_TOOL_ARGUMENT_BYTES);
        assert_eq!(
            limits.max_total_tool_argument_bytes,
            MAX_TOOL_ARGUMENT_BYTES
        );
        assert_eq!(limits.max_tool_call_id_bytes, MAX_TOOL_FIELD_BYTES);
        assert_eq!(limits.max_tool_name_bytes, MAX_TOOL_FIELD_BYTES);
    }
}

// Keep this privacy fixture in the ABI module so it exercises the exact
// production system-message builder used by redrob_agent_propose.
#[cfg(test)]
mod privacy_regression {
    use redrob_core::{Command, Document, EMBEDDED_FONT_ID, Editor, LayerId, Pixel, TextContent};

    use super::{agent_layers_value, agent_system_prompt, document_value, layers_value};

    #[test]
    fn hosted_system_prompt_redacts_text_while_local_projection_retains_source() {
        const SECRET: &str = "SENTINEL-private-customer-copy";
        let mut editor = Editor::new(Document::new(16, 16).unwrap()).unwrap();
        editor
            .execute(Command::AddTextNode {
                id: LayerId::new(),
                name: "Private text".into(),
                parent: None,
                sibling_index: 1,
                text: TextContent {
                    text: SECRET.into(),
                    font_family: "metadata family".into(),
                    font_size: 11.5,
                    color: Pixel::rgba(1, 2, 3, 4),
                    origin_x: 7.25,
                    origin_y: 9.5,
                    font_id: EMBEDDED_FONT_ID.into(),
                },
            })
            .unwrap();

        let local = serde_json::to_string(&layers_value(&editor)).unwrap();
        assert!(local.contains(SECRET));
        let safe_layers = agent_layers_value(&editor);
        let prompt = agent_system_prompt(&document_value(&editor), &safe_layers).unwrap();
        assert!(!prompt.contains(SECRET));
        assert!(!prompt.contains("preview"));
        assert!(prompt.contains(&format!("\"character_count\":{}", SECRET.chars().count())));
        assert!(prompt.contains(&format!("\"byte_count\":{}", SECRET.len())));
        assert!(prompt.contains("metadata family"));
    }
}
