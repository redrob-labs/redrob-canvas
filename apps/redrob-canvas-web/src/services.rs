// SPDX-License-Identifier: MIT OR Apache-2.0

//! The platform services each hosted editor needs, in the browser. The desktop module is
//! apps/redrob-canvas/src/services.rs; this one keeps the names `shell.rs` calls and answers them
//! the way the upstream web shells do (`crafts/*/apps/*-web/src/web.rs`):
//!
//! - file pickers through `rfd::AsyncFileDialog`; the chosen files' CONTENTS arrive asynchronously
//!   through each editor's `Services::inbox`, which the editor drains on its next frame;
//! - saves and exports are browser downloads of the bytes under the suggested name;
//! - preferences live in `localStorage` under one key per editor;
//! - there is no config directory, no fonts folder and no recovery folder (every "dir" is `None`).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use wasm_bindgen::JsCast as _;

use crate::routing;
use crate::shell::Mode;

/// Desktop app ID (kept for the shell's title and the about box).
pub const APP_ID: &str = "ai.redrob.canvas";
/// The window title and the brand text in the top bar.
pub const APP_NAME: &str = "Redrob Canvas";

/// No settings directory in a browser: every path-based service is off.
pub fn config_dir() -> Option<PathBuf> {
    None
}

/// Preferences are kept (in `localStorage`).
pub fn prefs_enabled() -> bool {
    true
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

fn storage_get(key: &str) -> Option<String> {
    local_storage()?.get_item(key).ok().flatten()
}

fn storage_set(key: &str, text: &str) -> Result<(), String> {
    local_storage()
        .ok_or("no localStorage")?
        .set_item(key, text)
        .map_err(|e| format!("{e:?}"))
}

/// Triggers a browser download of `bytes` named after the last component of `path`.
pub fn download(path: &str, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let name = std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "redrob-canvas".into());
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("application/octet-stream");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
    let a: web_sys::HtmlAnchorElement = document
        .create_element("a")
        .map_err(js)?
        .dyn_into()
        .map_err(|_| "not an anchor")?;
    a.set_href(&url);
    a.set_download(&name);
    a.style().set_property("display", "none").map_err(js)?;
    let body = document.body().ok_or("no body")?;
    body.append_child(&a).map_err(js)?;
    a.click();
    a.remove();
    // Revoke after the click has been dispatched; the download keeps its own reference.
    let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
        web_sys::Url::revoke_object_url(&url).ok();
    });
    window
        .set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 10_000)
        .map_err(js)?;
    Ok(())
}

/// One inbox per editor, process-wide, so files dropped on the window (src/web.rs) reach the
/// editor the shell routes them to; the editor's own `Services::inbox` is the same `Arc`.
static PHOTO_INBOX: OnceLock<photocraft_ui_egui::Inbox> = OnceLock::new();
static VECTOR_INBOX: OnceLock<vectorcraft_ui_egui::Inbox> = OnceLock::new();
static LAYOUT_INBOX: OnceLock<designcraft_ui_egui::Inbox> = OnceLock::new();

pub fn photo_inbox() -> photocraft_ui_egui::Inbox {
    PHOTO_INBOX
        .get_or_init(|| Arc::new(Mutex::new(Vec::new())))
        .clone()
}

pub fn vector_inbox() -> vectorcraft_ui_egui::Inbox {
    VECTOR_INBOX
        .get_or_init(|| Arc::new(Mutex::new(Vec::new())))
        .clone()
}

pub fn layout_inbox() -> designcraft_ui_egui::Inbox {
    LAYOUT_INBOX
        .get_or_init(|| Arc::new(Mutex::new(Vec::new())))
        .clone()
}

/// Delivers a dropped file's contents to the Photo or Vector editor through its inbox. Layout
/// takes [`deliver_layout`] (its inbox carries the import's purpose and target document).
pub fn deliver(mode: Mode, name: String, bytes: Vec<u8>) {
    match mode {
        Mode::Photo => photo_inbox()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((name, bytes)),
        Mode::Vector => vector_inbox()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((name, bytes)),
        Mode::Layout => log::warn!("{name}: Layout takes deliver_layout"),
    }
}

/// Delivers a dropped file to the Layout editor as `request` (`DesignApp::import_request`).
pub fn deliver_layout(request: designcraft_ui_egui::ImportRequest, name: String, bytes: Vec<u8>) {
    layout_inbox()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(designcraft_ui_egui::ImportedFile {
            request,
            name,
            bytes,
        });
}

/// The download's name for a suggested path: its file name, else `fallback`.
fn download_name(suggested: &str, fallback: &str) -> String {
    std::path::Path::new(suggested)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| fallback.to_owned())
}

// ---------------------------------------------------------------------------------------------
// Photo (photocraft)
// ---------------------------------------------------------------------------------------------

pub mod photo {
    use super::*;
    use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
    use photocraft_doc::Document;
    use photocraft_ui_egui::{FileDialogAnswer, FileDialogRequest, Services};

    const PREFS_KEY: &str = "redrob-canvas.photo.preferences";

    /// The services the Photo editor runs with in the browser (the upstream web shell's set,
    /// without its IndexedDB preset store and served fonts).
    pub fn services() -> Services {
        let inbox = photo_inbox();
        Services {
            import: Some(Box::new(
                |name: &str, bytes: &[u8], max_svg_group_depth: usize| {
                    photocraft_io::import_with_svg_group_depth(name, bytes, max_svg_group_depth)
                        .map(|r| (r.document, r.warnings))
                        .map_err(|e| e.to_string())
                },
            )),
            export: Some(Box::new(
                |doc: &Document, path: &str, settings: &photocraft_ui_egui::ExportSettings| {
                    photocraft_io::export(doc, path, &settings.io_options())
                        .map(|r| (r.bytes, r.warnings))
                        .map_err(|e| e.to_string())
                },
            )),
            file_dialog: Some(Box::new(|request, _parent, reply| match request {
                // The browser's picker hands over contents, not paths; File > Open asks for
                // several files (`multiple`), each opened in its own tab.
                FileDialogRequest::Open {
                    multiple,
                    extensions,
                    ..
                } => wasm_bindgen_futures::spawn_local(async move {
                    let dialog = match extensions {
                        Some(exts) => {
                            rfd::AsyncFileDialog::new().add_filter("Supported Files", &exts)
                        }
                        None => rfd::AsyncFileDialog::new()
                            .add_filter("All Formats", routing::extensions_for(Mode::Photo)),
                    };
                    let answer = if multiple {
                        match dialog.pick_files().await {
                            Some(picked) if !picked.is_empty() => {
                                let mut files = Vec::with_capacity(picked.len());
                                for file in picked {
                                    files.push((file.file_name(), file.read().await));
                                }
                                Some(FileDialogAnswer::ContentsMany(files))
                            }
                            _ => None,
                        }
                    } else {
                        match dialog.pick_file().await {
                            Some(file) => Some(FileDialogAnswer::Contents(
                                file.file_name(),
                                file.read().await,
                            )),
                            None => None,
                        }
                    };
                    reply.send(answer);
                }),
                // Browser folder handles cannot be filesystem paths for engine commands.
                FileDialogRequest::Folder { .. } => reply.send(None),
                // No save dialog on the web: the suggested name becomes the download name.
                FileDialogRequest::Save { suggested } => {
                    reply.send(Some(FileDialogAnswer::SaveTo(download_name(
                        &suggested,
                        "untitled.pcraft",
                    ))));
                }
            })),
            write: Some(Box::new(|path: &str, bytes: &[u8]| download(path, bytes))),
            encode_png: Some(Box::new(|w, h, rgba| {
                let img = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.to_vec())
                    .map_err(|e| e.to_string())?;
                photocraft_codecs::encode(
                    &img,
                    photocraft_codecs::Format::Png,
                    &EncodeOptions::default(),
                )
                .map_err(|e| e.to_string())
            })),
            inbox: Some(inbox),
            load_prefs: Some(Box::new(|| storage_get(PREFS_KEY))),
            save_prefs: Some(Box::new(|text: &str| storage_set(PREFS_KEY, text))),
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Vector (vectorcraft)
// ---------------------------------------------------------------------------------------------

pub mod vector {
    use super::*;
    use vectorcraft_ui_egui::{Services, VectorcraftApp};

    const PREFS_KEY: &str = "redrob-canvas.vector.ui";

    /// The services the Vector editor runs with in the browser: File > Open through the picker
    /// into the inbox, saves and exports as downloads.
    pub fn services() -> Services {
        let inbox = vector_inbox();
        let open_inbox = inbox.clone();
        Services {
            open_async: Some(Box::new(move || {
                let inbox = open_inbox.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let dialog = rfd::AsyncFileDialog::new()
                        .add_filter("Vector documents", routing::extensions_for(Mode::Vector));
                    let Some(file) = dialog.pick_file().await else {
                        return;
                    };
                    let bytes = file.read().await;
                    inbox
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push((file.file_name(), bytes));
                });
            })),
            // A download that couldn't be started fails the save or export it is for.
            download: Some(Box::new(|name: &str, bytes: &[u8]| {
                download(name, bytes).inspect_err(|e| log::error!("download of {name} failed: {e}"))
            })),
            inbox: Some(inbox),
            ..Default::default()
        }
    }

    /// No fonts folder in a browser; the editor's built-in fonts stand.
    pub fn install_fonts() {}

    /// No user library folders and no recovery folder in a browser.
    pub fn attach_dirs(_app: &mut VectorcraftApp) {}

    pub fn load_prefs(app: &mut VectorcraftApp) {
        if let Some(text) = storage_get(PREFS_KEY)
            && let Ok(ui) = serde_json::from_str::<vectorcraft_ui_egui::UiState>(&text)
        {
            app.ui = ui.sanitized();
        }
        vectorcraft_ui_egui::prefs_dialog::restore(app);
    }

    pub fn save_prefs(app: &VectorcraftApp) {
        let mut ui = app.ui.clone();
        ui.engine_prefs = app.session.prefs.to_json();
        if let Ok(text) = serde_json::to_string(&ui)
            && let Err(e) = storage_set(PREFS_KEY, &text)
        {
            log::warn!("couldn't save the Vector preferences: {e}");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Layout (designcraft)
// ---------------------------------------------------------------------------------------------

pub mod layout {
    use super::*;
    use designcraft_ui_egui::{DesignApp, ImportRequest, ImportedFile, Services};

    const PREFS_KEY: &str = "redrob-canvas.layout.ui";
    const ENGINE_PREFS_KEY: &str = "redrob-canvas.layout.prefs";

    /// No recovery folder in a browser.
    pub fn recovery_dir() -> Option<PathBuf> {
        None
    }

    /// The services the Layout editor runs with in the browser.
    pub fn services() -> Services {
        let inbox = layout_inbox();
        let open_inbox = inbox.clone();
        Services {
            open_async: Some(Box::new(move |request: ImportRequest| {
                let inbox = open_inbox.clone();
                let dialog = match request.purpose.as_str() {
                    "swatches" => {
                        rfd::AsyncFileDialog::new().add_filter("Swatch Exchange (ASE)", &["ase"])
                    }
                    "place" => rfd::AsyncFileDialog::new()
                        .add_filter("Graphics", routing::extensions_for(Mode::Photo)),
                    _ => rfd::AsyncFileDialog::new()
                        .add_filter("Layout documents", routing::extensions_for(Mode::Layout)),
                };
                wasm_bindgen_futures::spawn_local(async move {
                    let Some(file) = dialog.pick_file().await else {
                        return;
                    };
                    let bytes = file.read().await;
                    inbox
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(ImportedFile {
                            request,
                            name: file.file_name(),
                            bytes,
                        });
                });
            })),
            // Exports ask for a name; the browser decides where the download goes.
            pick_save: Some(Box::new(|name: &str| Some(download_name(name, "untitled")))),
            write: Some(Box::new(|name: &str, bytes: &[u8]| download(name, bytes))),
            download: Some(Box::new(|name: &str, bytes: &[u8]| {
                if let Err(e) = download(name, bytes) {
                    log::error!("download of {name} failed: {e}");
                }
            })),
            inbox: Some(inbox),
            ..Default::default()
        }
    }

    pub fn load_prefs(app: &mut DesignApp) {
        if let Some(text) = storage_get(PREFS_KEY)
            && let Ok(ui) = designcraft_ui_egui::UiState::from_preferences_json(text.as_bytes())
        {
            app.ui = ui;
        }
        if let Some(text) = storage_get(ENGINE_PREFS_KEY)
            && let Ok(prefs) = serde_json::from_str::<designcraft_engine::Prefs>(&text)
        {
            app.session.prefs = prefs;
        }
    }

    pub fn save_prefs(app: &DesignApp) {
        if let Ok(text) = serde_json::to_string(&app.ui)
            && let Err(e) = storage_set(PREFS_KEY, &text)
        {
            log::warn!("couldn't save the Layout preferences: {e}");
        }
        if let Ok(text) = serde_json::to_string(&app.session.prefs)
            && let Err(e) = storage_set(ENGINE_PREFS_KEY, &text)
        {
            log::warn!("couldn't save the Layout engine preferences: {e}");
        }
    }

    /// The tab closing is the browser's `beforeunload` (src/web.rs); nothing to preserve to disk.
    pub fn preserve_documents_on_exit(app: &mut DesignApp) -> Result<(), String> {
        app.finish_confirmed_quit()?;
        if app.session.documents().iter().any(|d| d.is_dirty()) {
            return Err(
                "Recovery is unavailable in the browser; unsaved documents were not preserved"
                    .into(),
            );
        }
        Ok(())
    }
}
