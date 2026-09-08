// SPDX-License-Identifier: GPL-3.0-or-later

use std::ffi::c_char;
use std::io::Write;
use std::mem::{align_of, offset_of, size_of};
use std::process::{Command, Stdio};
use std::ptr;

use redrob_ffi::*;
use serde_json::Value;

unsafe fn take_buffer(buffer: RedrobBuffer) -> Vec<u8> {
    let bytes = if buffer.data.is_null() {
        Vec::new()
    } else {
        // SAFETY: The FFI call returned a valid immutable buffer that remains
        // alive until redrob_buffer_free below.
        unsafe { std::slice::from_raw_parts(buffer.data, buffer.len) }.to_vec()
    };
    // SAFETY: This buffer is returned exactly once to its owning allocator.
    unsafe { redrob_buffer_free(buffer) };
    bytes
}

unsafe fn last_error() -> String {
    let length = unsafe { redrob_last_error_copy(ptr::null_mut(), 0) };
    let mut bytes = vec![0_u8; length + 1];
    unsafe { redrob_last_error_copy(bytes.as_mut_ptr().cast::<c_char>(), bytes.len()) };
    String::from_utf8(bytes[..length].to_vec()).unwrap()
}

fn compile_header_contract(compiler: &str, language: &str, standard: &str, source: &str) {
    let include = format!("{}/include", env!("CARGO_MANIFEST_DIR"));
    let mut child = Command::new(compiler)
        .args([
            standard,
            "-Wall",
            "-Wextra",
            "-Werror",
            "-fsyntax-only",
            "-x",
            language,
            "-I",
            &include,
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("failed to start {compiler}: {error}"));
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{compiler} rejected the ABI contract:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn null_and_malformed_inputs_are_reported_without_unwinding() {
    let mut output = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_execute_json(ptr::null_mut(), ptr::null(), 0, &mut output) },
        REDROB_ERROR
    );
    assert!(unsafe { last_error() }.contains("editor handle is null"));

    let mut editor = ptr::null_mut();
    assert_eq!(
        unsafe { redrob_editor_create(2, 2, &mut editor) },
        REDROB_OK
    );
    assert!(!editor.is_null());

    let malformed = b"{not json";
    assert_eq!(
        unsafe {
            redrob_editor_execute_json(editor, malformed.as_ptr(), malformed.len(), &mut output)
        },
        REDROB_ERROR
    );
    assert!(unsafe { last_error() }.contains("invalid command JSON"));

    assert_eq!(
        unsafe { redrob_editor_execute_json(editor, ptr::null(), 4, &mut output) },
        REDROB_ERROR
    );
    assert!(unsafe { last_error() }.contains("null"));

    assert_eq!(
        unsafe { redrob_editor_document_json(editor, ptr::null_mut()) },
        REDROB_ERROR
    );
    unsafe { redrob_editor_destroy(editor) };
}

#[test]
fn command_json_limit_is_checked_before_deserialization() {
    let mut editor = ptr::null_mut();
    assert_eq!(
        unsafe { redrob_editor_create(1, 1, &mut editor) },
        REDROB_OK
    );
    let mut output = RedrobBuffer::default();

    let exact = vec![b' '; MAX_COMMAND_JSON_BYTES];
    assert_eq!(
        unsafe { redrob_editor_execute_json(editor, exact.as_ptr(), exact.len(), &mut output) },
        REDROB_ERROR
    );
    assert!(unsafe { last_error() }.contains("invalid command JSON"));

    output = RedrobBuffer {
        data: ptr::dangling_mut::<u8>(),
        len: 1,
    };
    let over = vec![b' '; MAX_COMMAND_JSON_BYTES + 1];
    assert_eq!(
        unsafe { redrob_editor_execute_json(editor, over.as_ptr(), over.len(), &mut output) },
        REDROB_ERROR
    );
    assert!(output.data.is_null());
    assert_eq!(output.len, 0);
    assert!(unsafe { last_error() }.contains("1048576-byte limit"));

    unsafe { redrob_editor_destroy(editor) };
}

#[test]
fn command_and_history_playback_transitions_are_failure_atomic_over_ffi() {
    let mut editor = ptr::null_mut();
    assert_eq!(
        unsafe { redrob_editor_create(1, 1, &mut editor) },
        REDROB_OK
    );

    let state = || {
        let mut output = RedrobBuffer::default();
        assert_eq!(
            unsafe { redrob_editor_state_json(editor, &mut output) },
            REDROB_OK
        );
        unsafe { take_buffer(output) }
    };
    let render = || {
        let mut snapshot = RedrobRenderSnapshot::default();
        assert_eq!(
            unsafe { redrob_editor_render_rgba(editor, &mut snapshot) },
            REDROB_OK
        );
        let generation = snapshot.generation;
        let pixels = unsafe { take_buffer(snapshot.rgba) };
        (generation, pixels)
    };
    let set_playing = |playing| {
        let mut changes = RedrobBuffer::default();
        assert_eq!(
            unsafe { redrob_editor_set_playing(editor, playing, &mut changes) },
            REDROB_OK
        );
        unsafe { redrob_buffer_free(changes) };
    };

    set_playing(true);
    let before_rejection = state();
    let before_render = render();
    let rejected = br#"{"type":"add_frame","id":0,"index":1}"#;
    let mut changes = RedrobBuffer {
        data: ptr::dangling_mut::<u8>(),
        len: 17,
    };
    assert_eq!(
        unsafe {
            redrob_editor_execute_json(editor, rejected.as_ptr(), rejected.len(), &mut changes)
        },
        REDROB_ERROR
    );
    assert!(changes.data.is_null());
    assert_eq!(changes.len, 0);
    assert_eq!(state(), before_rejection);
    assert_eq!(render(), before_render);

    let fill = br#"{"type":"fill","color":{"r":1,"g":2,"b":3,"a":255}}"#;
    assert_eq!(
        unsafe { redrob_editor_execute_json(editor, fill.as_ptr(), fill.len(), &mut changes) },
        REDROB_OK
    );
    let applied: Value = serde_json::from_slice(&unsafe { take_buffer(changes) }).unwrap();
    let applied_state: Value = serde_json::from_slice(&state()).unwrap();
    assert_eq!(applied["generation"], before_render.0 + 1);
    assert_eq!(applied["timeline_changed"], true);
    assert_eq!(applied["navigation_changed"], true);
    assert_eq!(applied_state["timeline"]["playing"], false);
    assert_eq!(applied_state["timeline"]["current_frame"], 0);
    assert_eq!(applied_state["document"]["can_undo"], true);
    assert_eq!(applied_state["document"]["can_redo"], false);

    set_playing(true);
    let generation_before_undo = render().0;
    assert_eq!(
        unsafe { redrob_editor_undo(editor, &mut changes) },
        REDROB_OK
    );
    let undone: Value = serde_json::from_slice(&unsafe { take_buffer(changes) }).unwrap();
    let undone_state: Value = serde_json::from_slice(&state()).unwrap();
    assert_eq!(undone["generation"], generation_before_undo + 1);
    assert_eq!(undone["timeline_changed"], true);
    assert_eq!(undone["navigation_changed"], true);
    assert_eq!(undone_state["timeline"]["playing"], false);
    assert_eq!(undone_state["timeline"]["current_frame"], 0);
    assert_eq!(undone_state["document"]["can_undo"], false);
    assert_eq!(undone_state["document"]["can_redo"], true);

    set_playing(true);
    let before_unavailable_undo = state();
    let before_unavailable_undo_render = render();
    assert_eq!(
        unsafe { redrob_editor_undo(editor, &mut changes) },
        REDROB_ERROR
    );
    assert!(changes.data.is_null());
    assert_eq!(changes.len, 0);
    assert_eq!(state(), before_unavailable_undo);
    assert_eq!(render(), before_unavailable_undo_render);

    let generation_before_redo = before_unavailable_undo_render.0;
    assert_eq!(
        unsafe { redrob_editor_redo(editor, &mut changes) },
        REDROB_OK
    );
    let redone: Value = serde_json::from_slice(&unsafe { take_buffer(changes) }).unwrap();
    assert_eq!(redone["generation"], generation_before_redo + 1);
    assert_eq!(redone["timeline_changed"], true);
    assert_eq!(redone["navigation_changed"], true);

    set_playing(true);
    let before_unavailable_redo = state();
    let before_unavailable_redo_render = render();
    assert_eq!(
        unsafe { redrob_editor_redo(editor, &mut changes) },
        REDROB_ERROR
    );
    assert!(changes.data.is_null());
    assert_eq!(changes.len, 0);
    assert_eq!(state(), before_unavailable_redo);
    assert_eq!(render(), before_unavailable_redo_render);

    unsafe { redrob_editor_destroy(editor) };
}

#[test]
fn execute_render_project_and_png_roundtrip_end_to_end() {
    let mut editor = ptr::null_mut();
    assert_eq!(
        unsafe { redrob_editor_create(2, 1, &mut editor) },
        REDROB_OK
    );

    let command = br#"{"type":"fill","color":{"r":20,"g":40,"b":60,"a":255}}"#;
    let mut changes = RedrobBuffer::default();
    assert_eq!(
        unsafe {
            redrob_editor_execute_json(editor, command.as_ptr(), command.len(), &mut changes)
        },
        REDROB_OK
    );
    let changes_json: Value = serde_json::from_slice(&unsafe { take_buffer(changes) }).unwrap();
    assert_eq!(changes_json["generation"], 1);

    let mut snapshot = RedrobRenderSnapshot::default();
    assert_eq!(
        unsafe { redrob_editor_render_rgba(editor, &mut snapshot) },
        REDROB_OK
    );
    assert_eq!(
        (snapshot.width, snapshot.height, snapshot.stride),
        (2, 1, 8)
    );
    assert_eq!(snapshot.generation, 1);
    assert_eq!(
        unsafe { take_buffer(snapshot.rgba) },
        [20, 40, 60, 255, 20, 40, 60, 255]
    );

    let mut project = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_save_rrg(editor, &mut project) },
        REDROB_OK
    );
    let project_bytes = unsafe { take_buffer(project) };

    let clear = br#"{"type":"clear"}"#;
    assert_eq!(
        unsafe { redrob_editor_execute_json(editor, clear.as_ptr(), clear.len(), &mut changes) },
        REDROB_OK
    );
    unsafe { redrob_buffer_free(changes) };
    assert_eq!(
        unsafe { redrob_editor_load_rrg(editor, project_bytes.as_ptr(), project_bytes.len()) },
        REDROB_OK
    );

    let mut png = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_export_png(editor, &mut png) },
        REDROB_OK
    );
    let png_bytes = unsafe { take_buffer(png) };
    assert!(png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(
        unsafe { redrob_editor_import_png(editor, png_bytes.as_ptr(), png_bytes.len()) },
        REDROB_OK
    );

    let mut document = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_document_json(editor, &mut document) },
        REDROB_OK
    );
    let document: Value = serde_json::from_slice(&unsafe { take_buffer(document) }).unwrap();
    assert_eq!(document["width"], 2);
    assert_eq!(document["height"], 1);
    assert_eq!(document["generation"], 0);

    unsafe { redrob_editor_destroy(editor) };
}

#[test]
fn generic_format_routes_roundtrip_with_structured_results_and_truthful_capabilities() {
    let mut editor = ptr::null_mut();
    assert_eq!(
        unsafe { redrob_editor_create(2, 1, &mut editor) },
        REDROB_OK
    );
    let fill = br#"{"type":"fill","color":{"r":12,"g":34,"b":56,"a":255}}"#;
    let mut changes = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_execute_json(editor, fill.as_ptr(), fill.len(), &mut changes) },
        REDROB_OK
    );
    unsafe { redrob_buffer_free(changes) };

    for (format, allow_loss, quality) in [
        ("rrg", false, 90),
        ("png", false, 90),
        ("webp", false, 90),
        ("jpeg", false, 83),
        ("ora", false, 90),
        ("svg", true, 90),
    ] {
        let export_options = serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "format": format,
            "loss_policy": if allow_loss { "allow_loss" } else { "reject_loss" },
            "jpeg_quality": quality,
            "jpeg_alpha": { "policy": "reject_non_opaque" }
        }))
        .unwrap();
        let mut bytes = RedrobBuffer::default();
        let mut result = RedrobBuffer::default();
        assert_eq!(
            unsafe {
                redrob_editor_export_file(
                    editor,
                    export_options.as_ptr(),
                    export_options.len(),
                    &mut bytes,
                    &mut result,
                )
            },
            REDROB_OK,
            "generic {format} export failed: {}",
            unsafe { last_error() }
        );
        let encoded = unsafe { take_buffer(bytes) };
        let export_result: Value = serde_json::from_slice(&unsafe { take_buffer(result) }).unwrap();
        assert_eq!(export_result["effective_format"], format);
        assert_eq!(export_result["detected_format"], format);
        assert_eq!(export_result["width"], 2);
        assert_eq!(export_result["height"], 1);
        assert_eq!(export_result["lossless"], format != "jpeg");
        assert_eq!(
            export_result["jpeg_quality"],
            if format == "jpeg" {
                Value::from(quality)
            } else {
                Value::Null
            }
        );
        if format == "svg" {
            assert_eq!(export_result["warnings"][0]["code"], "embedded_raster_data");
        }

        let import_options = serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "expected_format": format,
            "loss_policy": if format == "svg" { "allow_loss" } else { "reject_loss" },
            "max_input_bytes": redrob_core::MAX_FORMAT_INPUT_BYTES
        }))
        .unwrap();
        let mut import_result = RedrobBuffer::default();
        assert_eq!(
            unsafe {
                redrob_editor_import_file(
                    editor,
                    encoded.as_ptr(),
                    encoded.len(),
                    import_options.as_ptr(),
                    import_options.len(),
                    &mut import_result,
                )
            },
            REDROB_OK,
            "generic {format} import failed: {}",
            unsafe { last_error() }
        );
        let import_result: Value =
            serde_json::from_slice(&unsafe { take_buffer(import_result) }).unwrap();
        assert_eq!(import_result["detected_format"], format);
        assert_eq!(import_result["effective_format"], format);
        assert_eq!(import_result["width"], 2);
        assert_eq!(import_result["height"], 1);
    }

    let mut capabilities = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_ffi_capabilities_json(&mut capabilities) },
        REDROB_OK
    );
    let capabilities: Value =
        serde_json::from_slice(&unsafe { take_buffer(capabilities) }).unwrap();
    let formats = capabilities["formats"].as_array().unwrap();
    assert_eq!(
        formats
            .iter()
            .map(|entry| entry["format"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["rrg", "png", "jpeg", "webp", "ora", "svg"]
    );
    assert_eq!(capabilities["adapters"]["gegl"]["compiled"], false);
    assert_eq!(capabilities["adapters"]["gegl"]["ready"], false);
    assert_eq!(
        capabilities["adapters"]["gegl"]["operations"],
        serde_json::json!([])
    );
    assert_eq!(capabilities["adapters"]["krita"]["compiled"], false);
    assert_eq!(
        capabilities["adapters"]["krita"]["scaffold_compiled"],
        false
    );
    assert_eq!(capabilities["adapters"]["krita"]["ready"], false);
    assert_eq!(
        capabilities["adapters"]["krita"]["formats"],
        serde_json::json!([])
    );

    unsafe { redrob_editor_destroy(editor) };
}

#[test]
fn generic_options_are_strict_bounded_atomic_and_reset_every_output() {
    fn sentinel() -> RedrobBuffer {
        RedrobBuffer {
            data: ptr::dangling_mut::<u8>(),
            len: 7,
        }
    }
    fn assert_reset(buffer: RedrobBuffer) {
        assert!(buffer.data.is_null());
        assert_eq!(buffer.len, 0);
    }

    let mut editor = ptr::null_mut();
    assert_eq!(
        unsafe { redrob_editor_create(3, 2, &mut editor) },
        REDROB_OK
    );
    let mut before = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_state_json(editor, &mut before) },
        REDROB_OK
    );
    let before = unsafe { take_buffer(before) };

    let malformed = b"{";
    let mut result = sentinel();
    assert_eq!(
        unsafe {
            redrob_editor_import_file(
                editor,
                ptr::null(),
                0,
                malformed.as_ptr(),
                malformed.len(),
                &mut result,
            )
        },
        REDROB_ERROR
    );
    assert_reset(result);
    assert!(unsafe { last_error() }.contains("invalid format options JSON"));

    let unknown = br#"{"schema_version":1,"expected_format":"png","unknown":true}"#;
    result = sentinel();
    assert_eq!(
        unsafe {
            redrob_editor_import_file(
                editor,
                ptr::null(),
                0,
                unknown.as_ptr(),
                unknown.len(),
                &mut result,
            )
        },
        REDROB_ERROR
    );
    assert_reset(result);
    assert!(unsafe { last_error() }.contains("unknown field"));

    let oversized = vec![b' '; MAX_FORMAT_OPTIONS_JSON_BYTES + 1];
    result = sentinel();
    assert_eq!(
        unsafe {
            redrob_editor_import_file(
                editor,
                ptr::null(),
                0,
                oversized.as_ptr(),
                oversized.len(),
                &mut result,
            )
        },
        REDROB_ERROR
    );
    assert_reset(result);
    assert!(unsafe { last_error() }.contains("65536-byte limit"));

    let png = redrob_core::export_document(
        &redrob_core::Document::new(1, 1).unwrap(),
        redrob_core::FileFormat::Png,
        &redrob_core::ExportOptions::default(),
    )
    .unwrap()
    .into_bytes();
    let wrong = br#"{"schema_version":1,"expected_format":"jpeg"}"#;
    result = sentinel();
    assert_eq!(
        unsafe {
            redrob_editor_import_file(
                editor,
                png.as_ptr(),
                png.len(),
                wrong.as_ptr(),
                wrong.len(),
                &mut result,
            )
        },
        REDROB_ERROR
    );
    assert_reset(result);
    assert!(unsafe { last_error() }.contains("does not match expected"));

    let mut after = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_state_json(editor, &mut after) },
        REDROB_OK
    );
    assert_eq!(unsafe { take_buffer(after) }, before);

    let bad_export = br#"{"schema_version":1,"format":"jpeg","unexpected":0}"#;
    let mut bytes = sentinel();
    result = sentinel();
    assert_eq!(
        unsafe {
            redrob_editor_export_file(
                editor,
                bad_export.as_ptr(),
                bad_export.len(),
                &mut bytes,
                &mut result,
            )
        },
        REDROB_ERROR
    );
    assert_reset(bytes);
    assert_reset(result);

    result = sentinel();
    assert_eq!(
        unsafe {
            redrob_editor_export_file(
                editor,
                bad_export.as_ptr(),
                bad_export.len(),
                ptr::null_mut(),
                &mut result,
            )
        },
        REDROB_ERROR
    );
    assert_reset(result);
    bytes = sentinel();
    assert_eq!(
        unsafe {
            redrob_editor_export_file(
                editor,
                bad_export.as_ptr(),
                bad_export.len(),
                &mut bytes,
                ptr::null_mut(),
            )
        },
        REDROB_ERROR
    );
    assert_reset(bytes);

    let valid_export = br#"{"schema_version":1,"format":"png"}"#;
    let mut aliased = sentinel();
    let aliased_pointer: *mut RedrobBuffer = &mut aliased;
    assert_eq!(
        unsafe {
            redrob_editor_export_file(
                editor,
                valid_export.as_ptr(),
                valid_export.len(),
                aliased_pointer,
                aliased_pointer,
            )
        },
        REDROB_ERROR
    );
    assert_reset(aliased);
    assert!(unsafe { last_error() }.contains("must be distinct"));

    let mut capabilities = sentinel();
    assert_eq!(
        unsafe { redrob_ffi_capabilities_json(ptr::null_mut()) },
        REDROB_ERROR
    );
    assert_eq!(
        unsafe { redrob_ffi_capabilities_json(&mut capabilities) },
        REDROB_OK
    );
    unsafe { redrob_buffer_free(capabilities) };
    unsafe { redrob_editor_destroy(editor) };
}

#[test]
fn generic_jpeg_alpha_and_explicit_frame_export_are_nonmutating() {
    let mut editor = ptr::null_mut();
    assert_eq!(
        unsafe { redrob_editor_create(1, 1, &mut editor) },
        REDROB_OK
    );
    let reject = br#"{"schema_version":1,"format":"jpeg","jpeg_quality":91,"jpeg_alpha":{"policy":"reject_non_opaque"}}"#;
    let mut bytes = RedrobBuffer::default();
    let mut result = RedrobBuffer::default();
    assert_eq!(
        unsafe {
            redrob_editor_export_file(
                editor,
                reject.as_ptr(),
                reject.len(),
                &mut bytes,
                &mut result,
            )
        },
        REDROB_ERROR
    );
    assert!(bytes.data.is_null() && result.data.is_null());
    assert!(unsafe { last_error() }.contains("explicit opaque matte"));

    let flatten = br#"{"schema_version":1,"format":"jpeg","jpeg_quality":91,"jpeg_alpha":{"policy":"flatten","matte":{"r":10,"g":20,"b":30,"a":255}}}"#;
    assert_eq!(
        unsafe {
            redrob_editor_export_file(
                editor,
                flatten.as_ptr(),
                flatten.len(),
                &mut bytes,
                &mut result,
            )
        },
        REDROB_OK
    );
    let jpeg = unsafe { take_buffer(bytes) };
    assert!(jpeg.starts_with(&[0xff, 0xd8, 0xff]));
    let jpeg_result: Value = serde_json::from_slice(&unsafe { take_buffer(result) }).unwrap();
    assert_eq!(jpeg_result["jpeg_quality"], 91);
    assert_eq!(jpeg_result["lossless"], false);
    assert_eq!(jpeg_result["warnings"][0]["code"], "flattened_alpha");
    assert_eq!(jpeg_result["warnings"][0]["matte"]["a"], 255);

    let invalid_matte = br#"{"schema_version":1,"format":"jpeg","jpeg_alpha":{"policy":"flatten","matte":{"r":0,"g":0,"b":0,"a":254}}}"#;
    assert_eq!(
        unsafe {
            redrob_editor_export_file(
                editor,
                invalid_matte.as_ptr(),
                invalid_matte.len(),
                &mut bytes,
                &mut result,
            )
        },
        REDROB_ERROR
    );
    assert!(unsafe { last_error() }.contains("matte must be opaque"));
    unsafe { redrob_editor_destroy(editor) };

    let alternate = redrob_core::FrameId::new(7);
    let mut builder = redrob_core::DocumentImportBuilder::new(1, 1).unwrap();
    builder
        .timeline(
            vec![redrob_core::FrameId::DEFAULT, alternate],
            10.0,
            redrob_core::FrameId::DEFAULT,
            redrob_core::PlaybackMetadata {
                range_start: redrob_core::FrameId::DEFAULT,
                range_end: alternate,
                looping: false,
                playing: false,
            },
        )
        .unwrap();
    builder
        .push_node(redrob_core::ImportNode::raster(
            "animated",
            vec![
                redrob_core::RasterCel::new(redrob_core::FrameId::DEFAULT, vec![255, 0, 0, 255]),
                redrob_core::RasterCel::new(alternate, vec![0, 255, 0, 255]),
            ],
        ))
        .unwrap();
    let project = redrob_core::save_project(&builder.build().unwrap()).unwrap();
    assert_eq!(
        unsafe { redrob_editor_create(1, 1, &mut editor) },
        REDROB_OK
    );
    assert_eq!(
        unsafe { redrob_editor_load_rrg(editor, project.as_ptr(), project.len()) },
        REDROB_OK
    );
    let edit = br#"{"type":"fill","color":{"r":200,"g":0,"b":0,"a":255}}"#;
    let mut changes = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_execute_json(editor, edit.as_ptr(), edit.len(), &mut changes) },
        REDROB_OK
    );
    unsafe { redrob_buffer_free(changes) };
    let mut before = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_state_json(editor, &mut before) },
        REDROB_OK
    );
    let before = unsafe { take_buffer(before) };
    let frame_export =
        br#"{"schema_version":1,"format":"png","frame":7,"loss_policy":"allow_loss"}"#;
    assert_eq!(
        unsafe {
            redrob_editor_export_file(
                editor,
                frame_export.as_ptr(),
                frame_export.len(),
                &mut bytes,
                &mut result,
            )
        },
        REDROB_OK
    );
    let png = unsafe { take_buffer(bytes) };
    let result: Value = serde_json::from_slice(&unsafe { take_buffer(result) }).unwrap();
    assert_eq!(result["frame"], 7);
    assert_eq!(result["warnings"][0]["code"], "omitted_frames");
    let imported =
        redrob_core::import_document(&png, &redrob_core::ImportOptions::default()).unwrap();
    assert_eq!(imported.document().layers()[0].pixels(), [0, 255, 0, 255]);
    let mut after = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_state_json(editor, &mut after) },
        REDROB_OK
    );
    assert_eq!(unsafe { take_buffer(after) }, before);
    unsafe { redrob_editor_destroy(editor) };
}

#[test]
fn last_error_is_thread_local() {
    let first = std::thread::spawn(|| {
        let mut output = RedrobBuffer::default();
        unsafe { redrob_editor_undo(ptr::null_mut(), &mut output) };
        unsafe { last_error() }
    });
    let second = std::thread::spawn(|| {
        let mut editor = ptr::null_mut();
        unsafe { redrob_editor_create(0, 1, &mut editor) };
        unsafe { last_error() }
    });
    assert!(first.join().unwrap().contains("editor handle is null"));
    assert!(second.join().unwrap().contains("dimensions"));
}

#[test]
fn agent_propose_rejects_null_malformed_empty_and_oversized_inputs_offline() {
    let key = b"test-key";
    let prompt = b"add a layer";
    let mut output = RedrobBuffer::default();
    assert_eq!(
        unsafe {
            redrob_agent_propose(
                ptr::null_mut(),
                key.as_ptr(),
                key.len(),
                prompt.as_ptr(),
                prompt.len(),
                &mut output,
            )
        },
        REDROB_ERROR
    );
    assert!(unsafe { last_error() }.contains("editor handle is null"));

    let mut editor = ptr::null_mut();
    assert_eq!(
        unsafe { redrob_editor_create(2, 2, &mut editor) },
        REDROB_OK
    );

    assert_eq!(
        unsafe {
            redrob_agent_propose(
                editor,
                key.as_ptr(),
                key.len(),
                prompt.as_ptr(),
                prompt.len(),
                ptr::null_mut(),
            )
        },
        REDROB_ERROR
    );
    assert!(unsafe { last_error() }.contains("output buffer pointer is null"));

    output = RedrobBuffer {
        data: ptr::dangling_mut::<u8>(),
        len: 1,
    };
    assert_eq!(
        unsafe {
            redrob_agent_propose(
                editor,
                ptr::null(),
                0,
                prompt.as_ptr(),
                prompt.len(),
                &mut output,
            )
        },
        REDROB_ERROR
    );
    assert!(output.data.is_null());
    assert_eq!(output.len, 0);
    assert!(unsafe { last_error() }.contains("API key must not be empty"));

    let invalid_utf8 = [0xff_u8];
    assert_eq!(
        unsafe {
            redrob_agent_propose(
                editor,
                invalid_utf8.as_ptr(),
                invalid_utf8.len(),
                prompt.as_ptr(),
                prompt.len(),
                &mut output,
            )
        },
        REDROB_ERROR
    );
    assert!(unsafe { last_error() }.contains("API key is not valid UTF-8"));
    assert_eq!(
        unsafe {
            redrob_agent_propose(
                editor,
                key.as_ptr(),
                key.len(),
                invalid_utf8.as_ptr(),
                invalid_utf8.len(),
                &mut output,
            )
        },
        REDROB_ERROR
    );
    assert!(unsafe { last_error() }.contains("prompt is not valid UTF-8"));

    let blank = b" \n\t ";
    assert_eq!(
        unsafe {
            redrob_agent_propose(
                editor,
                key.as_ptr(),
                key.len(),
                blank.as_ptr(),
                blank.len(),
                &mut output,
            )
        },
        REDROB_ERROR
    );
    assert!(unsafe { last_error() }.contains("prompt must not be empty"));

    let oversized = vec![b'x'; 16 * 1024 + 1];
    assert_eq!(
        unsafe {
            redrob_agent_propose(
                editor,
                key.as_ptr(),
                key.len(),
                oversized.as_ptr(),
                oversized.len(),
                &mut output,
            )
        },
        REDROB_ERROR
    );
    assert!(unsafe { last_error() }.contains("16384-byte limit"));

    let secret_with_newline = b"super-secret-key\n";
    assert_eq!(
        unsafe {
            redrob_agent_propose(
                editor,
                secret_with_newline.as_ptr(),
                secret_with_newline.len(),
                prompt.as_ptr(),
                prompt.len(),
                &mut output,
            )
        },
        REDROB_ERROR
    );
    let error = unsafe { last_error() };
    assert_eq!(error, "Redrob agent configuration is invalid");
    assert!(!error.contains("super-secret-key"));

    unsafe { redrob_editor_destroy(editor) };
}

#[test]
fn owned_selection_mask_snapshot_survives_later_edits() {
    let mut editor = ptr::null_mut();
    assert_eq!(
        unsafe { redrob_editor_create(3, 2, &mut editor) },
        REDROB_OK
    );

    let mut initial = RedrobSelectionMaskSnapshot::default();
    assert_eq!(
        unsafe { redrob_editor_selection_mask(editor, &mut initial) },
        REDROB_OK
    );
    assert_eq!(
        (
            initial.width,
            initial.height,
            initial.stride,
            initial.generation
        ),
        (3, 2, 3, 0)
    );
    assert_eq!(initial.active, 0);
    assert_eq!(unsafe { take_buffer(initial.mask) }, [0; 6]);

    let select = br#"{"type":"select_rectangle","rect":{"x":1,"y":0,"width":2,"height":1},"mode":"replace"}"#;
    let mut changes = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_execute_json(editor, select.as_ptr(), select.len(), &mut changes) },
        REDROB_OK
    );
    unsafe { redrob_buffer_free(changes) };

    let mut selected = RedrobSelectionMaskSnapshot::default();
    assert_eq!(
        unsafe { redrob_editor_selection_mask(editor, &mut selected) },
        REDROB_OK
    );
    assert_eq!(selected.generation, 1);
    assert_eq!(selected.active, 1);

    let clear = br#"{"type":"clear_selection"}"#;
    assert_eq!(
        unsafe { redrob_editor_execute_json(editor, clear.as_ptr(), clear.len(), &mut changes) },
        REDROB_OK
    );
    unsafe { redrob_buffer_free(changes) };

    // The first snapshot is Rust-owned independently of later editor mutation.
    assert_eq!(
        unsafe { take_buffer(selected.mask) },
        [0, 255, 255, 0, 0, 0]
    );

    let mut cleared = RedrobSelectionMaskSnapshot::default();
    assert_eq!(
        unsafe { redrob_editor_selection_mask(editor, &mut cleared) },
        REDROB_OK
    );
    assert_eq!(cleared.generation, 2);
    assert_eq!(cleared.active, 0);
    assert_eq!(unsafe { take_buffer(cleared.mask) }, [0; 6]);

    unsafe { redrob_editor_destroy(editor) };
}

#[test]
fn every_owned_output_is_zeroed_before_failure() {
    fn sentinel() -> RedrobBuffer {
        RedrobBuffer {
            data: ptr::dangling_mut::<u8>(),
            len: 99,
        }
    }
    fn assert_reset(buffer: RedrobBuffer) {
        assert!(buffer.data.is_null());
        assert_eq!(buffer.len, 0);
    }

    let mut output = sentinel();
    assert_eq!(
        unsafe { redrob_editor_execute_json(ptr::null_mut(), ptr::null(), 0, &mut output) },
        REDROB_ERROR
    );
    assert_reset(output);

    for operation in [
        redrob_editor_undo as unsafe extern "C" fn(*mut RedrobEditor, *mut RedrobBuffer) -> i32,
        redrob_editor_redo,
        redrob_editor_document_json,
        redrob_editor_layers_json,
        redrob_editor_timeline_json,
        redrob_editor_state_json,
        redrob_editor_save_rrg,
        redrob_editor_export_png,
    ] {
        output = sentinel();
        assert_eq!(
            unsafe { operation(ptr::null_mut(), &mut output) },
            REDROB_ERROR
        );
        assert_reset(output);
    }

    let options = br#"{"schema_version":1,"format":"png"}"#;
    let mut result = sentinel();
    output = sentinel();
    assert_eq!(
        unsafe {
            redrob_editor_export_file(
                ptr::null_mut(),
                options.as_ptr(),
                options.len(),
                &mut output,
                &mut result,
            )
        },
        REDROB_ERROR
    );
    assert_reset(output);
    assert_reset(result);

    let import_options = br#"{"schema_version":1,"expected_format":"png"}"#;
    result = sentinel();
    assert_eq!(
        unsafe {
            redrob_editor_import_file(
                ptr::null_mut(),
                ptr::null(),
                0,
                import_options.as_ptr(),
                import_options.len(),
                &mut result,
            )
        },
        REDROB_ERROR
    );
    assert_reset(result);

    output = sentinel();
    assert_eq!(
        unsafe { redrob_editor_set_current_frame(ptr::null_mut(), 0, &mut output) },
        REDROB_ERROR
    );
    assert_reset(output);
    output = sentinel();
    assert_eq!(
        unsafe { redrob_editor_set_playing(ptr::null_mut(), false, &mut output) },
        REDROB_ERROR
    );
    assert_reset(output);
    output = sentinel();
    assert_eq!(
        unsafe { redrob_editor_advance_playback(ptr::null_mut(), &mut output) },
        REDROB_ERROR
    );
    assert_reset(output);

    let mut render = RedrobRenderSnapshot {
        rgba: sentinel(),
        width: 1,
        height: 2,
        stride: 3,
        generation: 4,
    };
    assert_eq!(
        unsafe { redrob_editor_render_rgba(ptr::null_mut(), &mut render) },
        REDROB_ERROR
    );
    assert!(render.rgba.data.is_null());
    assert_eq!(
        (
            render.rgba.len,
            render.width,
            render.height,
            render.stride,
            render.generation
        ),
        (0, 0, 0, 0, 0)
    );

    let mut selection = RedrobSelectionMaskSnapshot {
        mask: sentinel(),
        width: 1,
        height: 2,
        stride: 3,
        generation: 4,
        active: 1,
    };
    assert_eq!(
        unsafe { redrob_editor_selection_mask(ptr::null_mut(), &mut selection) },
        REDROB_ERROR
    );
    assert!(selection.mask.data.is_null());
    assert_eq!(
        (
            selection.mask.len,
            selection.width,
            selection.height,
            selection.stride,
            selection.generation,
            selection.active,
        ),
        (0, 0, 0, 0, 0, 0)
    );
}

#[test]
fn rust_exports_and_c_header_remain_at_abi_v2_parity() {
    let header = include_str!("../include/redrob_ffi.h");
    assert_eq!(redrob_ffi_abi_version(), 2);
    assert_eq!(REDROB_FFI_ABI_VERSION, 2);
    assert!(header.contains("#define REDROB_FFI_ABI_VERSION 2u"));

    for symbol in [
        "redrob_ffi_abi_version",
        "redrob_last_error_copy",
        "redrob_buffer_free",
        "redrob_editor_create",
        "redrob_editor_destroy",
        "redrob_agent_propose",
        "redrob_editor_execute_json",
        "redrob_editor_undo",
        "redrob_editor_redo",
        "redrob_editor_document_json",
        "redrob_editor_layers_json",
        "redrob_editor_timeline_json",
        "redrob_editor_state_json",
        "redrob_editor_set_current_frame",
        "redrob_editor_set_playing",
        "redrob_editor_advance_playback",
        "redrob_editor_render_rgba",
        "redrob_editor_selection_mask",
        "redrob_ffi_capabilities_json",
        "redrob_editor_import_file",
        "redrob_editor_export_file",
        "redrob_editor_load_rrg",
        "redrob_editor_save_rrg",
        "redrob_editor_import_png",
        "redrob_editor_export_png",
    ] {
        assert!(header.contains(symbol), "C header is missing {symbol}");
    }
    for field in [
        "typedef struct RedrobSelectionMaskSnapshot",
        "RedrobBuffer mask;",
        "uint32_t width;",
        "uint32_t height;",
        "uint32_t stride;",
        "uint64_t generation;",
        "uint8_t active;",
    ] {
        assert!(header.contains(field), "C header is missing `{field}`");
    }

    let _: unsafe extern "C" fn(*mut RedrobEditor, *mut RedrobSelectionMaskSnapshot) -> i32 =
        redrob_editor_selection_mask;
    let _: unsafe extern "C" fn(*mut RedrobBuffer) -> i32 = redrob_ffi_capabilities_json;
    let _: unsafe extern "C" fn(
        *mut RedrobEditor,
        *const u8,
        usize,
        *const u8,
        usize,
        *mut RedrobBuffer,
    ) -> i32 = redrob_editor_import_file;
    let _: unsafe extern "C" fn(
        *mut RedrobEditor,
        *const u8,
        usize,
        *mut RedrobBuffer,
        *mut RedrobBuffer,
    ) -> i32 = redrob_editor_export_file;
}

#[test]
fn compiled_c_and_cpp_layouts_match_rust_abi() {
    let c_source = format!(
        r#"
#include "redrob_ffi.h"
_Static_assert(sizeof(RedrobBuffer) == {buffer_size}, "RedrobBuffer size");
_Static_assert(_Alignof(RedrobBuffer) == {buffer_align}, "RedrobBuffer alignment");
_Static_assert(offsetof(RedrobBuffer, data) == {buffer_data}, "RedrobBuffer.data");
_Static_assert(offsetof(RedrobBuffer, len) == {buffer_len}, "RedrobBuffer.len");
_Static_assert(sizeof(RedrobRenderSnapshot) == {render_size}, "RedrobRenderSnapshot size");
_Static_assert(_Alignof(RedrobRenderSnapshot) == {render_align}, "RedrobRenderSnapshot alignment");
_Static_assert(offsetof(RedrobRenderSnapshot, rgba) == {render_rgba}, "RedrobRenderSnapshot.rgba");
_Static_assert(offsetof(RedrobRenderSnapshot, width) == {render_width}, "RedrobRenderSnapshot.width");
_Static_assert(offsetof(RedrobRenderSnapshot, height) == {render_height}, "RedrobRenderSnapshot.height");
_Static_assert(offsetof(RedrobRenderSnapshot, stride) == {render_stride}, "RedrobRenderSnapshot.stride");
_Static_assert(offsetof(RedrobRenderSnapshot, generation) == {render_generation}, "RedrobRenderSnapshot.generation");
_Static_assert(sizeof(RedrobSelectionMaskSnapshot) == {selection_size}, "RedrobSelectionMaskSnapshot size");
_Static_assert(_Alignof(RedrobSelectionMaskSnapshot) == {selection_align}, "RedrobSelectionMaskSnapshot alignment");
_Static_assert(offsetof(RedrobSelectionMaskSnapshot, mask) == {selection_mask}, "RedrobSelectionMaskSnapshot.mask");
_Static_assert(offsetof(RedrobSelectionMaskSnapshot, width) == {selection_width}, "RedrobSelectionMaskSnapshot.width");
_Static_assert(offsetof(RedrobSelectionMaskSnapshot, height) == {selection_height}, "RedrobSelectionMaskSnapshot.height");
_Static_assert(offsetof(RedrobSelectionMaskSnapshot, stride) == {selection_stride}, "RedrobSelectionMaskSnapshot.stride");
_Static_assert(offsetof(RedrobSelectionMaskSnapshot, generation) == {selection_generation}, "RedrobSelectionMaskSnapshot.generation");
_Static_assert(offsetof(RedrobSelectionMaskSnapshot, active) == {selection_active}, "RedrobSelectionMaskSnapshot.active");
typedef int32_t (*SelectionMaskFn)(RedrobEditor *, RedrobSelectionMaskSnapshot *);
typedef int32_t (*CapabilitiesFn)(RedrobBuffer *);
typedef int32_t (*ImportFileFn)(RedrobEditor *, const uint8_t *, size_t, const uint8_t *, size_t, RedrobBuffer *);
typedef int32_t (*ExportFileFn)(RedrobEditor *, const uint8_t *, size_t, RedrobBuffer *, RedrobBuffer *);
_Static_assert(_Generic(&redrob_editor_selection_mask, SelectionMaskFn: 1, default: 0), "redrob_editor_selection_mask signature");
_Static_assert(_Generic(&redrob_ffi_capabilities_json, CapabilitiesFn: 1, default: 0), "redrob_ffi_capabilities_json signature");
_Static_assert(_Generic(&redrob_editor_import_file, ImportFileFn: 1, default: 0), "redrob_editor_import_file signature");
_Static_assert(_Generic(&redrob_editor_export_file, ExportFileFn: 1, default: 0), "redrob_editor_export_file signature");
"#,
        buffer_size = size_of::<RedrobBuffer>(),
        buffer_align = align_of::<RedrobBuffer>(),
        buffer_data = offset_of!(RedrobBuffer, data),
        buffer_len = offset_of!(RedrobBuffer, len),
        render_size = size_of::<RedrobRenderSnapshot>(),
        render_align = align_of::<RedrobRenderSnapshot>(),
        render_rgba = offset_of!(RedrobRenderSnapshot, rgba),
        render_width = offset_of!(RedrobRenderSnapshot, width),
        render_height = offset_of!(RedrobRenderSnapshot, height),
        render_stride = offset_of!(RedrobRenderSnapshot, stride),
        render_generation = offset_of!(RedrobRenderSnapshot, generation),
        selection_size = size_of::<RedrobSelectionMaskSnapshot>(),
        selection_align = align_of::<RedrobSelectionMaskSnapshot>(),
        selection_mask = offset_of!(RedrobSelectionMaskSnapshot, mask),
        selection_width = offset_of!(RedrobSelectionMaskSnapshot, width),
        selection_height = offset_of!(RedrobSelectionMaskSnapshot, height),
        selection_stride = offset_of!(RedrobSelectionMaskSnapshot, stride),
        selection_generation = offset_of!(RedrobSelectionMaskSnapshot, generation),
        selection_active = offset_of!(RedrobSelectionMaskSnapshot, active),
    );
    compile_header_contract("cc", "c", "-std=c11", &c_source);

    let cpp_source = format!(
        r#"
#include <type_traits>
#include "redrob_ffi.h"
static_assert(sizeof(RedrobBuffer) == {buffer_size});
static_assert(alignof(RedrobBuffer) == {buffer_align});
static_assert(offsetof(RedrobBuffer, data) == {buffer_data});
static_assert(offsetof(RedrobBuffer, len) == {buffer_len});
static_assert(sizeof(RedrobRenderSnapshot) == {render_size});
static_assert(alignof(RedrobRenderSnapshot) == {render_align});
static_assert(offsetof(RedrobRenderSnapshot, rgba) == {render_rgba});
static_assert(offsetof(RedrobRenderSnapshot, width) == {render_width});
static_assert(offsetof(RedrobRenderSnapshot, height) == {render_height});
static_assert(offsetof(RedrobRenderSnapshot, stride) == {render_stride});
static_assert(offsetof(RedrobRenderSnapshot, generation) == {render_generation});
static_assert(sizeof(RedrobSelectionMaskSnapshot) == {selection_size});
static_assert(alignof(RedrobSelectionMaskSnapshot) == {selection_align});
static_assert(offsetof(RedrobSelectionMaskSnapshot, mask) == {selection_mask});
static_assert(offsetof(RedrobSelectionMaskSnapshot, width) == {selection_width});
static_assert(offsetof(RedrobSelectionMaskSnapshot, height) == {selection_height});
static_assert(offsetof(RedrobSelectionMaskSnapshot, stride) == {selection_stride});
static_assert(offsetof(RedrobSelectionMaskSnapshot, generation) == {selection_generation});
static_assert(offsetof(RedrobSelectionMaskSnapshot, active) == {selection_active});
using SelectionMaskFn = int32_t (*)(RedrobEditor *, RedrobSelectionMaskSnapshot *);
using CapabilitiesFn = int32_t (*)(RedrobBuffer *);
using ImportFileFn = int32_t (*)(RedrobEditor *, const uint8_t *, size_t, const uint8_t *, size_t, RedrobBuffer *);
using ExportFileFn = int32_t (*)(RedrobEditor *, const uint8_t *, size_t, RedrobBuffer *, RedrobBuffer *);
static_assert(std::is_same_v<decltype(&redrob_editor_selection_mask), SelectionMaskFn>);
static_assert(std::is_same_v<decltype(&redrob_ffi_capabilities_json), CapabilitiesFn>);
static_assert(std::is_same_v<decltype(&redrob_editor_import_file), ImportFileFn>);
static_assert(std::is_same_v<decltype(&redrob_editor_export_file), ExportFileFn>);
"#,
        buffer_size = size_of::<RedrobBuffer>(),
        buffer_align = align_of::<RedrobBuffer>(),
        buffer_data = offset_of!(RedrobBuffer, data),
        buffer_len = offset_of!(RedrobBuffer, len),
        render_size = size_of::<RedrobRenderSnapshot>(),
        render_align = align_of::<RedrobRenderSnapshot>(),
        render_rgba = offset_of!(RedrobRenderSnapshot, rgba),
        render_width = offset_of!(RedrobRenderSnapshot, width),
        render_height = offset_of!(RedrobRenderSnapshot, height),
        render_stride = offset_of!(RedrobRenderSnapshot, stride),
        render_generation = offset_of!(RedrobRenderSnapshot, generation),
        selection_size = size_of::<RedrobSelectionMaskSnapshot>(),
        selection_align = align_of::<RedrobSelectionMaskSnapshot>(),
        selection_mask = offset_of!(RedrobSelectionMaskSnapshot, mask),
        selection_width = offset_of!(RedrobSelectionMaskSnapshot, width),
        selection_height = offset_of!(RedrobSelectionMaskSnapshot, height),
        selection_stride = offset_of!(RedrobSelectionMaskSnapshot, stride),
        selection_generation = offset_of!(RedrobSelectionMaskSnapshot, generation),
        selection_active = offset_of!(RedrobSelectionMaskSnapshot, active),
    );
    compile_header_contract("c++", "c++", "-std=c++17", &cpp_source);
}

#[test]
fn semantic_v2_project_renders_through_ffi_with_wire_defaults() {
    let project = redrob_core::save_project(&redrob_core::Document::new(2, 1).unwrap()).unwrap();
    let mut value: Value = serde_json::from_slice(&project).unwrap();
    value["document"]["nodes"][0]["content"] = serde_json::json!({
        "kind": "text",
        "text": {
            "text": "semantic",
            "font_family": "Redrob Sans",
            "font_size": 12.0,
            "color": { "r": 255, "g": 255, "b": 255, "a": 255 }
        }
    });
    let project = serde_json::to_vec(&value).unwrap();

    let mut editor = ptr::null_mut();
    assert_eq!(
        unsafe { redrob_editor_create(1, 1, &mut editor) },
        REDROB_OK
    );
    assert_eq!(
        unsafe { redrob_editor_load_rrg(editor, project.as_ptr(), project.len()) },
        REDROB_OK
    );

    let mut snapshot = RedrobRenderSnapshot {
        rgba: RedrobBuffer {
            data: ptr::dangling_mut::<u8>(),
            len: 1,
        },
        width: 99,
        height: 99,
        stride: 99,
        generation: 99,
    };
    assert_eq!(
        unsafe { redrob_editor_render_rgba(editor, &mut snapshot) },
        REDROB_OK
    );
    assert!(!snapshot.rgba.data.is_null());
    assert_eq!(snapshot.rgba.len, 8);
    assert_eq!(
        (
            snapshot.width,
            snapshot.height,
            snapshot.stride,
            snapshot.generation
        ),
        (2, 1, 8, 0)
    );
    unsafe { redrob_buffer_free(snapshot.rgba) };

    unsafe { redrob_editor_destroy(editor) };
}

#[test]
fn semantic_commands_capabilities_rasterize_and_history_work_end_to_end() {
    let mut editor = ptr::null_mut();
    assert_eq!(
        unsafe { redrob_editor_create(16, 16, &mut editor) },
        REDROB_OK
    );
    let execute = |value: Value| {
        let bytes = serde_json::to_vec(&value).unwrap();
        let mut changes = RedrobBuffer::default();
        let status = unsafe {
            redrob_editor_execute_json(editor, bytes.as_ptr(), bytes.len(), &mut changes)
        };
        let payload = unsafe { take_buffer(changes) };
        (status, payload)
    };
    let state = || {
        let mut output = RedrobBuffer::default();
        assert_eq!(
            unsafe { redrob_editor_state_json(editor, &mut output) },
            REDROB_OK
        );
        serde_json::from_slice::<Value>(&unsafe { take_buffer(output) }).unwrap()
    };
    let render = || {
        let mut snapshot = RedrobRenderSnapshot::default();
        assert_eq!(
            unsafe { redrob_editor_render_rgba(editor, &mut snapshot) },
            REDROB_OK
        );
        let pixels = unsafe { take_buffer(snapshot.rgba) };
        (snapshot.generation, pixels)
    };

    let text_id = "00000000-0000-0000-0000-000000000201";
    assert_eq!(
        execute(serde_json::json!({
            "type": "add_text_node", "id": text_id, "name": "Caption", "parent": null,
            "sibling_index": 1,
            "text": { "text": "A", "font_family": "metadata", "font_size": 8.0,
                "color": { "r": 255, "g": 0, "b": 0, "a": 255 },
                "origin_x": 1.0, "origin_y": 1.0, "font_id": "font8x8-basic-0.3.1" }
        }))
        .0,
        REDROB_OK
    );
    let projected = state();
    let text = projected["layers"]["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == text_id)
        .unwrap();
    assert_eq!(text["capabilities"]["can_edit_text"], true);
    assert_eq!(text["capabilities"]["can_edit_vector"], false);
    assert_eq!(text["capabilities"]["can_rasterize"], true);
    assert_eq!(text["capabilities"]["is_semantic"], true);
    assert_eq!(text["semantic"]["preview"], "A");
    assert_eq!(text["semantic"]["text"], "A");
    assert_eq!(text["semantic"]["font_id"], "font8x8-basic-0.3.1");
    assert_eq!(text["semantic"]["font_family"], "metadata");
    assert_eq!(text["semantic"]["font_size"], 8.0);
    assert_eq!(text["semantic"]["origin_x"], 1.0);
    assert_eq!(text["semantic"]["origin_y"], 1.0);
    assert_eq!(
        text["semantic"]["color"],
        serde_json::json!({
            "r": 255, "g": 0, "b": 0, "a": 255
        })
    );
    let (_, before) = render();

    assert_eq!(
        execute(serde_json::json!({ "type": "rasterize_semantic_node", "id": text_id })).0,
        REDROB_OK
    );
    let projected = state();
    let raster = projected["layers"]["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == text_id)
        .unwrap();
    assert_eq!(raster["kind"], "raster");
    assert_eq!(raster["capabilities"]["can_edit_raster"], true);
    assert_eq!(render().1, before);

    let mut changes = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_undo(editor, &mut changes) },
        REDROB_OK
    );
    unsafe { redrob_buffer_free(changes) };
    let projected = state();
    assert_eq!(
        projected["layers"]["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == text_id)
            .unwrap()["kind"],
        "text"
    );
    let mut changes = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_redo(editor, &mut changes) },
        REDROB_OK
    );
    unsafe { redrob_buffer_free(changes) };
    assert_eq!(render().1, before);

    let generation = state()["document"]["generation"].as_u64().unwrap();
    let invalid = execute(serde_json::json!({
        "type": "set_text_content", "id": text_id,
        "text": { "text": "é", "font_family": "metadata", "font_size": 8.0,
            "color": { "r": 0, "g": 0, "b": 0, "a": 255 },
            "origin_x": 0.0, "origin_y": 0.0, "font_id": "font8x8-basic-0.3.1" }
    }));
    assert_eq!(invalid.0, REDROB_ERROR);
    assert!(invalid.1.is_empty());
    assert_eq!(state()["document"]["generation"], generation);
    unsafe { redrob_editor_destroy(editor) };
}

#[test]
fn hierarchy_mask_commands_and_additive_json_summaries_work_end_to_end() {
    let mut editor = ptr::null_mut();
    assert_eq!(
        unsafe { redrob_editor_create(2, 1, &mut editor) },
        REDROB_OK
    );

    let snapshot_json = |editor| {
        let mut output = RedrobBuffer::default();
        assert_eq!(
            unsafe { redrob_editor_layers_json(editor, &mut output) },
            REDROB_OK
        );
        serde_json::from_slice::<Value>(&unsafe { take_buffer(output) }).unwrap()
    };
    let execute = |editor, value: Value| {
        let bytes = serde_json::to_vec(&value).unwrap();
        let mut changes = RedrobBuffer::default();
        assert_eq!(
            unsafe {
                redrob_editor_execute_json(editor, bytes.as_ptr(), bytes.len(), &mut changes)
            },
            REDROB_OK,
            "{}",
            unsafe { last_error() }
        );
        serde_json::from_slice::<Value>(&unsafe { take_buffer(changes) }).unwrap()
    };

    let initial = snapshot_json(editor);
    assert_eq!(initial["schema_version"], 2);
    let raster = initial["active_node_id"].as_str().unwrap().to_owned();
    let group = "00000000-0000-0000-0000-000000000099";
    execute(
        editor,
        serde_json::json!({
            "type": "fill",
            "color": { "r": 20, "g": 40, "b": 60, "a": 255 }
        }),
    );
    execute(
        editor,
        serde_json::json!({
            "type": "add_group",
            "id": group,
            "name": "Paint",
            "parent": null,
            "sibling_index": 1
        }),
    );
    execute(
        editor,
        serde_json::json!({
            "type": "move_node",
            "id": raster,
            "parent": group,
            "sibling_index": 0
        }),
    );
    execute(
        editor,
        serde_json::json!({
            "type": "select_rectangle",
            "rect": { "x": 1, "y": 0, "width": 1, "height": 1 },
            "mode": "replace"
        }),
    );
    execute(
        editor,
        serde_json::json!({ "type": "raster_mask_from_selection", "id": group }),
    );

    let layers = snapshot_json(editor);
    assert_eq!(layers["active_node_id"], group);
    let nodes = layers["layers"].as_array().unwrap();
    assert_eq!(nodes.len(), 2);
    let raster_node = nodes.iter().find(|node| node["id"] == raster).unwrap();
    assert_eq!(raster_node["kind"], "raster");
    assert_eq!(raster_node["parent_id"], group);
    assert_eq!(raster_node["depth"], 1);
    assert_eq!(raster_node["capabilities"]["can_edit_raster"], true);
    let group_node = nodes.iter().find(|node| node["id"] == group).unwrap();
    assert_eq!(group_node["kind"], "group");
    assert_eq!(group_node["has_mask"], true);
    assert_eq!(group_node["mask_enabled"], true);
    assert_eq!(group_node["group"]["child_count"], 1);
    assert_eq!(group_node["capabilities"]["supports_pass_through"], false);

    let mut document = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_document_json(editor, &mut document) },
        REDROB_OK
    );
    let document: Value = serde_json::from_slice(&unsafe { take_buffer(document) }).unwrap();
    assert_eq!(document["schema_version"], 2);
    assert_eq!(document["node_count"], 2);
    assert_eq!(document["active_node_id"], group);

    let mut render = RedrobRenderSnapshot::default();
    assert_eq!(
        unsafe { redrob_editor_render_rgba(editor, &mut render) },
        REDROB_OK
    );
    assert_eq!(
        unsafe { take_buffer(render.rgba) },
        [0, 0, 0, 0, 20, 40, 60, 255]
    );

    let invalid = serde_json::to_vec(&serde_json::json!({
        "type": "remove_layer",
        "id": group
    }))
    .unwrap();
    let mut changes = RedrobBuffer::default();
    assert_eq!(
        unsafe {
            redrob_editor_execute_json(editor, invalid.as_ptr(), invalid.len(), &mut changes)
        },
        REDROB_ERROR
    );
    assert!(unsafe { last_error() }.contains("not empty"));
    assert!(changes.data.is_null());

    unsafe { redrob_editor_destroy(editor) };
}

#[test]
fn timeline_projection_and_navigation_are_generation_coherent_and_history_neutral() {
    let mut editor = ptr::null_mut();
    assert_eq!(
        unsafe { redrob_editor_create(1, 1, &mut editor) },
        REDROB_OK
    );

    let execute = |json: &[u8]| {
        let mut output = RedrobBuffer::default();
        assert_eq!(
            unsafe { redrob_editor_execute_json(editor, json.as_ptr(), json.len(), &mut output) },
            REDROB_OK
        );
        unsafe { take_buffer(output) }
    };
    execute(br#"{"type":"add_frame","id":1,"index":1}"#);
    execute(br#"{"type":"set_playback_range","start":0,"end":1}"#);
    execute(br#"{"type":"fill","color":{"r":1,"g":2,"b":3,"a":255}}"#);
    let mut history = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_undo(editor, &mut history) },
        REDROB_OK
    );
    unsafe { take_buffer(history) };

    let mut before_buffer = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_document_json(editor, &mut before_buffer) },
        REDROB_OK
    );
    let before: Value = serde_json::from_slice(&unsafe { take_buffer(before_buffer) }).unwrap();
    assert_eq!(before["can_undo"], true);
    assert_eq!(before["can_redo"], true);

    let mut changes = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_set_current_frame(editor, 1, &mut changes) },
        REDROB_OK
    );
    let navigation: Value = serde_json::from_slice(&unsafe { take_buffer(changes) }).unwrap();
    assert_eq!(navigation["timeline_changed"], true);
    assert_eq!(navigation["navigation_changed"], true);
    assert_eq!(navigation["canvas_changed"], true);

    let mut timeline_buffer = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_timeline_json(editor, &mut timeline_buffer) },
        REDROB_OK
    );
    let timeline: Value = serde_json::from_slice(&unsafe { take_buffer(timeline_buffer) }).unwrap();
    assert_eq!(timeline["frames"].as_array().unwrap().len(), 2);
    assert_eq!(timeline["frames"][1]["id"], 1);
    assert_eq!(timeline["frames"][1]["current"], true);
    assert!(timeline["generation"].as_u64().unwrap() > before["generation"].as_u64().unwrap());

    let mut after_buffer = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_document_json(editor, &mut after_buffer) },
        REDROB_OK
    );
    let after: Value = serde_json::from_slice(&unsafe { take_buffer(after_buffer) }).unwrap();
    assert_eq!(after["can_undo"], before["can_undo"]);
    assert_eq!(after["can_redo"], before["can_redo"]);

    let mut state_buffer = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_state_json(editor, &mut state_buffer) },
        REDROB_OK
    );
    let state: Value = serde_json::from_slice(&unsafe { take_buffer(state_buffer) }).unwrap();
    assert_eq!(state["generation"], state["document"]["generation"]);
    assert_eq!(state["generation"], state["layers"]["generation"]);
    assert_eq!(state["generation"], state["timeline"]["generation"]);

    let mut invalid = RedrobBuffer {
        data: ptr::dangling_mut::<u8>(),
        len: 1,
    };
    assert_eq!(
        unsafe { redrob_editor_set_current_frame(editor, 99, &mut invalid) },
        REDROB_ERROR
    );
    assert!(invalid.data.is_null());
    assert_eq!(invalid.len, 0);

    let mut playing = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_set_playing(editor, true, &mut playing) },
        REDROB_OK
    );
    unsafe { take_buffer(playing) };
    let mut tick = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_advance_playback(editor, &mut tick) },
        REDROB_OK
    );
    unsafe { take_buffer(tick) };

    unsafe { redrob_editor_destroy(editor) };
}

#[test]
fn ffi_projects_authoritative_rectangle_source_and_disables_arbitrary_vector_editing() {
    let mut editor = ptr::null_mut();
    assert_eq!(
        unsafe { redrob_editor_create(32, 32, &mut editor) },
        REDROB_OK
    );
    let execute = |value: Value| {
        let bytes = serde_json::to_vec(&value).unwrap();
        let mut changes = RedrobBuffer::default();
        let status = unsafe {
            redrob_editor_execute_json(editor, bytes.as_ptr(), bytes.len(), &mut changes)
        };
        unsafe { redrob_buffer_free(changes) };
        status
    };
    let layers = || {
        let mut output = RedrobBuffer::default();
        assert_eq!(
            unsafe { redrob_editor_layers_json(editor, &mut output) },
            REDROB_OK
        );
        serde_json::from_slice::<Value>(&unsafe { take_buffer(output) }).unwrap()
    };
    let rectangle = "00000000-0000-0000-0000-000000000301";
    assert_eq!(
        execute(serde_json::json!({
            "type": "add_vector_node", "id": rectangle, "name": "Rectangle", "parent": null,
            "sibling_index": 1,
            "vector": {"paths": [{
                "commands": [
                    {"type":"move_to", "x":2.25, "y":3.5},
                    {"type":"line_to", "x":12.75, "y":3.5},
                    {"type":"line_to", "x":12.75, "y":11.25},
                    {"type":"line_to", "x":2.25, "y":11.25},
                    {"type":"close"}
                ],
                "fill":{"r":11,"g":22,"b":33,"a":44},
                "stroke":{"color":{"r":55,"g":66,"b":77,"a":88},"width":2.5},
                "fill_rule":"non_zero"
            }]}
        })),
        REDROB_OK
    );
    let snapshot = layers();
    let node = snapshot["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == rectangle)
        .unwrap();
    assert_eq!(node["capabilities"]["can_edit_vector"], true);
    assert_eq!(node["semantic"]["rectangle_recognized"], true);
    assert_eq!(node["semantic"]["rectangle"]["x"], 2.25);
    assert_eq!(node["semantic"]["rectangle"]["y"], 3.5);
    assert_eq!(node["semantic"]["rectangle"]["width"], 10.5);
    assert_eq!(node["semantic"]["rectangle"]["height"], 7.75);
    assert_eq!(node["semantic"]["rectangle"]["stroke_width"], 2.5);

    let arbitrary = "00000000-0000-0000-0000-000000000302";
    assert_eq!(
        execute(serde_json::json!({
            "type": "add_vector_node", "id": arbitrary, "name": "Triangle", "parent": null,
            "sibling_index": 2,
            "vector": {"paths": [{
                "commands": [
                    {"type":"move_to", "x":0.0, "y":0.0},
                    {"type":"line_to", "x":8.0, "y":0.0},
                    {"type":"line_to", "x":4.0, "y":8.0},
                    {"type":"close"}
                ],
                "fill":{"r":1,"g":2,"b":3,"a":255}
            }]}
        })),
        REDROB_OK
    );
    let snapshot = layers();
    let node = snapshot["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == arbitrary)
        .unwrap();
    assert_eq!(node["capabilities"]["can_edit_vector"], false);
    assert_eq!(node["semantic"]["rectangle_recognized"], false);

    for (id, name, paint) in [
        (
            "00000000-0000-0000-0000-000000000303",
            "Fill only",
            serde_json::json!({"fill":{"r":1,"g":2,"b":3,"a":255}}),
        ),
        (
            "00000000-0000-0000-0000-000000000304",
            "Stroke only",
            serde_json::json!({"stroke":{"color":{"r":4,"g":5,"b":6,"a":255},"width":1.5}}),
        ),
    ] {
        let mut path = serde_json::json!({
            "commands": [
                {"type":"move_to", "x":1.0, "y":1.0},
                {"type":"line_to", "x":5.0, "y":1.0},
                {"type":"line_to", "x":5.0, "y":4.0},
                {"type":"line_to", "x":1.0, "y":4.0},
                {"type":"close"}
            ],
            "fill_rule":"non_zero"
        });
        path.as_object_mut()
            .unwrap()
            .extend(paint.as_object().unwrap().clone());
        assert_eq!(
            execute(serde_json::json!({
                "type":"add_vector_node", "id":id, "name":name, "parent":null,
                "sibling_index":3, "vector":{"paths":[path]}
            })),
            REDROB_OK
        );
        let snapshot = layers();
        let node = snapshot["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == id)
            .unwrap();
        assert_eq!(node["capabilities"]["can_edit_vector"], false);
        assert_eq!(node["semantic"]["rectangle_recognized"], false);
    }
    unsafe { redrob_editor_destroy(editor) };
}

#[test]
fn ffi_over_budget_semantic_command_does_not_change_generation_or_history() {
    let mut editor = ptr::null_mut();
    assert_eq!(
        unsafe { redrob_editor_create(1024, 1024, &mut editor) },
        REDROB_OK
    );
    let mut commands = vec![serde_json::json!({"type":"move_to", "x":0.0, "y":0.0})];
    for index in 0..20 {
        commands.push(serde_json::json!({
            "type":"line_to",
            "x": if index % 2 == 0 { 1024.0 } else { 0.0 },
            "y": (index as f64 * 53.0).min(1024.0)
        }));
    }
    commands.push(serde_json::json!({"type":"close"}));
    let command = serde_json::to_vec(&serde_json::json!({
        "type":"add_vector_node", "id":"00000000-0000-0000-0000-000000000399",
        "name":"Over budget", "parent":null, "sibling_index":1,
        "vector":{"paths":[{"commands":commands,
            "fill":{"r":1,"g":2,"b":3,"a":255}, "fill_rule":"even_odd"}]}
    }))
    .unwrap();
    let mut changes = RedrobBuffer::default();
    assert_eq!(
        unsafe {
            redrob_editor_execute_json(editor, command.as_ptr(), command.len(), &mut changes)
        },
        REDROB_ERROR
    );
    unsafe { redrob_buffer_free(changes) };

    let mut state = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_state_json(editor, &mut state) },
        REDROB_OK
    );
    let state: Value = serde_json::from_slice(&unsafe { take_buffer(state) }).unwrap();
    assert_eq!(state["generation"], 0);
    assert_eq!(state["document"]["can_undo"], false);
    let mut undo = RedrobBuffer::default();
    assert_eq!(
        unsafe { redrob_editor_undo(editor, &mut undo) },
        REDROB_ERROR
    );
    unsafe { redrob_buffer_free(undo) };
    unsafe { redrob_editor_destroy(editor) };
}
