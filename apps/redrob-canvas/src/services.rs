//! Where Redrob Canvas keeps its per-user data, and the platform services each hosted editor
//! needs, ported (minimally) from the three upstream desktop binaries:
//!
//! - `crafts/photocraft/apps/photocraft/src/{app_dirs,services}.rs`
//! - `crafts/vectorcraft/apps/vectorcraft/src/{main,prefs_dir}.rs`
//! - `crafts/designcraft/apps/designcraft/src/{main,profile}.rs`
//!
//! One config directory for the whole app (see [`config_dir`]); each editor gets a folder of its
//! own under it (`photo/`, `vector/`, `layout/`) so their `ui.json` / `preferences.json` and
//! recovery stores never collide.
//!
//! TODO(control server): the upstream control / MCP servers
//! (`crafts/*/apps/*/src/control_server.rs`, `PhotocraftApp::with_control`,
//! `VectorcraftApp::with_control`, `DesignApp::with_control`) are not started by the shell yet.

use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

/// Desktop app ID: the Linux `.desktop` file name, hicolor icon name and Wayland app id.
pub const APP_ID: &str = "ai.redrob.canvas";
/// The window title and the brand text in the top bar.
pub const APP_NAME: &str = "Redrob Canvas";
/// `REDROB_CANVAS_CONFIG_DIR` overrides the platform directory (tests, agents, custom setups).
pub const CONFIG_DIR_ENV: &str = "REDROB_CANVAS_CONFIG_DIR";
/// `REDROB_CANVAS_NO_PREFS`: read and write nothing in the settings directory (tests, demos).
pub const NO_PREFS_ENV: &str = "REDROB_CANVAS_NO_PREFS";

/// Per-user settings directory: `REDROB_CANVAS_CONFIG_DIR`, else the platform convention
/// (macOS `~/Library/Application Support/Redrob Canvas`, Windows `%APPDATA%\Redrob Canvas`,
/// elsewhere `$XDG_CONFIG_HOME/redrob-canvas` or `~/.config/redrob-canvas`).
pub fn config_dir() -> Option<PathBuf> {
    config_dir_with(std::env::consts::OS, |key| std::env::var_os(key))
}

/// [`config_dir`] from an explicit environment, so tests can fake `HOME` / `APPDATA`.
pub fn config_dir_with(os: &str, env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(root) = env(CONFIG_DIR_ENV) {
        // An explicitly empty override must not fall back to the user's profile.
        return (!root.is_empty()).then(|| PathBuf::from(root));
    }
    let home = env("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    match os {
        "macos" => home.map(|h| h.join("Library/Application Support").join(APP_NAME)),
        "windows" => env("APPDATA")
            .filter(|a| !a.is_empty())
            .map(|a| PathBuf::from(a).join(APP_NAME)),
        _ => env("XDG_CONFIG_HOME")
            .filter(|x| !x.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".config")))
            .map(|c| c.join("redrob-canvas")),
    }
}

/// Runs without preferences neither read nor write them.
pub fn prefs_enabled() -> bool {
    std::env::var_os(NO_PREFS_ENV).is_none()
}

/// eframe's own persistence file (window geometry, panel sizes).
pub fn persistence_path() -> Option<PathBuf> {
    config_dir()
        .filter(|_| prefs_enabled())
        .map(|d| d.join("ui.ron"))
}

/// The editor folders under the config directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    Photo,
    Vector,
    Layout,
}

impl Editor {
    pub fn folder(self) -> &'static str {
        match self {
            Editor::Photo => "photo",
            Editor::Vector => "vector",
            Editor::Layout => "layout",
        }
    }
}

/// `<config dir>/<editor folder>`, created when it does not exist yet.
pub fn editor_dir(editor: Editor) -> Option<PathBuf> {
    let dir = config_dir()?.join(editor.folder());
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("couldn't create {}: {e}", dir.display());
    }
    Some(dir)
}

fn read_bytes(p: &str) -> Result<Vec<u8>, String> {
    std::fs::read(p).map_err(|e| e.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn open_url(url: &str) -> Result<(), String> {
    webbrowser::open(url).map(|_| ()).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------------
// Photo (photocraft)
// ---------------------------------------------------------------------------------------------

pub mod photo {
    use super::*;
    use photocraft_doc::Document;
    use photocraft_format::RecoveryStore;
    use photocraft_ui_egui::{
        FileDialogAnswer, FileDialogReply, FileDialogRequest, Recoverable, Services,
    };

    pub fn prefs_file() -> Option<PathBuf> {
        editor_dir(Editor::Photo).map(|d| d.join("preferences.json"))
    }

    pub fn presets_dir() -> Option<PathBuf> {
        editor_dir(Editor::Photo).map(|d| d.join("Presets"))
    }

    fn recovery_dir() -> Option<PathBuf> {
        editor_dir(Editor::Photo).map(|d| d.join("Recovery"))
    }

    fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
        photocraft_format::atomic_write(path, bytes).map_err(|e| e.to_string())
    }

    type SharedRecovery = Rc<RefCell<Option<RecoveryStore>>>;

    fn with_store<R>(
        store: &SharedRecovery,
        f: impl FnOnce(&mut RecoveryStore) -> R,
    ) -> Result<R, String> {
        let mut slot = store
            .try_borrow_mut()
            .map_err(|_| "crash recovery is busy".to_string())?;
        Ok(f(slot.as_mut().ok_or("no config directory")?))
    }

    /// Crash recovery: background incremental autosaves into `dir` (ported from upstream
    /// `services::recovery_services`).
    fn recovery_services(dir: Option<PathBuf>) -> Services {
        let recover_dir = dir.clone();
        let store: SharedRecovery = Rc::new(RefCell::new(dir.map(RecoveryStore::new)));
        let (s1, s2, s4) = (store.clone(), store.clone(), store.clone());
        Services {
            autosave: Some(Box::new(
                move |doc: &Arc<Document>, revision: u64, path: Option<&str>| {
                    with_store(&s1, |s| {
                        s.autosave_checked(doc, revision, path.map(str::to_string))
                    })?
                },
            )),
            autosave_results: Some(Box::new(move || {
                with_store(&s4, |s| s.take_completed()).unwrap_or_default()
            })),
            discard_autosave: Some(Box::new(move |id: u64| {
                let _ = with_store(&s2, |s| s.discard(id));
            })),
            recover: Some(Box::new(move || {
                let Some(dir) = recover_dir.as_ref() else {
                    return Vec::new();
                };
                photocraft_format::list_recovery(dir)
                    .into_iter()
                    .map(|e| Recoverable {
                        key: e.info.key.clone(),
                        name: e.info.document_name.clone(),
                        path: e.info.original_path.clone(),
                        load: Box::new(move || {
                            photocraft_format::recover(&e).map_err(|e| e.to_string())
                        }),
                    })
                    .collect()
            })),
            adopt_autosave: Some(Box::new(move |id: u64, key: &str| {
                let _ = with_store(&store, |s| s.adopt(id, key));
            })),
            ..Default::default()
        }
    }

    /// Everything File › Open reads (upstream `services::OPEN_EXTS`).
    const OPEN_EXTS: &[&str] = &[
        "pcraft", "pdn", "ora", "kra", "krz", "psd", "psb", "psdt", "png", "jpg", "jpeg", "tif",
        "tiff", "webp", "gif", "bmp", "tga", "ico", "qoi", "exr", "hdr", "pbm", "pgm", "ppm",
        "pam", "pfm", "pcx", "dds", "xpm", "fits", "fit", "fts", "dpx", "cin", "heic", "heif",
        "hif", "dng", "cr2", "cr3", "nef", "nrw", "arw", "pef", "orf", "rw2", "raf", "abr", "grd",
        "svg", "svgz", "aco", "ase", "kys", "af", "afdesign", "afphoto", "afpub",
    ];

    const SAVE_FILTERS: &[(&str, &[&str])] = &[
        ("PSD Document", &["psd", "psb"]),
        ("PDF", &["pdf"]),
        ("Redrob Canvas photo document", &["pcraft"]),
        ("OpenRaster", &["ora"]),
        ("PNG", &["png"]),
        ("JPEG", &["jpg"]),
        ("WebP", &["webp"]),
        ("TIFF", &["tif"]),
        ("Targa", &["tga"]),
        ("BMP", &["bmp"]),
        ("GIF", &["gif"]),
        ("ICO", &["ico"]),
        ("OpenEXR", &["exr"]),
        ("FITS", &["fits", "fit"]),
        ("DPX", &["dpx"]),
    ];

    /// Open dialog extensions that also match uppercase names on Linux's case-sensitive globs.
    fn open_filter_extensions(extensions: &[&str]) -> Vec<String> {
        let case_sensitive_globs = cfg!(all(unix, not(target_os = "macos")));
        let class = |c: char| {
            if c.is_ascii_alphabetic() {
                format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase())
            } else {
                c.to_string()
            }
        };
        extensions
            .iter()
            .map(|ext| {
                if case_sensitive_globs {
                    ext.chars().map(class).collect()
                } else {
                    ext.to_string()
                }
            })
            .collect()
    }

    /// Save filters with the suggested name's type first, so the dialog keeps that extension.
    fn save_filters(suggested: &str) -> Vec<(String, Vec<String>)> {
        let ext = Path::new(suggested)
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let mut v: Vec<(String, Vec<String>)> = SAVE_FILTERS
            .iter()
            .map(|(n, e)| (n.to_string(), e.iter().map(|e| e.to_string()).collect()))
            .collect();
        match v.iter().position(|(_, exts)| exts.contains(&ext)) {
            Some(i) => {
                let (name, mut exts) = v.remove(i);
                exts.retain(|e| *e != ext);
                exts.insert(0, ext);
                v.insert(0, (name, exts));
            }
            None if !ext.is_empty() => v.insert(0, (ext.to_ascii_uppercase(), vec![ext])),
            None => {}
        }
        v
    }

    /// Shows an Open or Save dialog on a thread of its own and answers through `reply`, so the
    /// event loop keeps running (ported from upstream `services::show_file_dialog`, without the
    /// window parent and the missing-portal probe).
    #[cfg(not(target_arch = "wasm32"))]
    fn show_file_dialog(
        request: FileDialogRequest,
        _parent: Option<&eframe::Frame>,
        reply: FileDialogReply,
    ) {
        let spawned = std::thread::Builder::new()
            .name("file dialog".into())
            .spawn(move || {
                let mut dialog = rfd::FileDialog::new();
                let answer = match request {
                    FileDialogRequest::Open {
                        multiple,
                        initial_dir,
                        extensions,
                    } => {
                        if let Some(dir) = initial_dir {
                            dialog = dialog.set_directory(dir);
                        }
                        dialog = match extensions {
                            Some(exts) => dialog.add_filter(
                                "Supported Files",
                                &open_filter_extensions(
                                    &exts.iter().map(String::as_str).collect::<Vec<_>>(),
                                ),
                            ),
                            None => dialog
                                .add_filter("All Formats", &open_filter_extensions(OPEN_EXTS))
                                .add_filter(
                                    "Redrob Canvas photo document",
                                    &open_filter_extensions(&["pcraft"]),
                                )
                                .add_filter("OpenRaster", &open_filter_extensions(&["ora"]))
                                .add_filter("SVG", &open_filter_extensions(&["svg", "svgz"])),
                        };
                        if multiple {
                            dialog.pick_files().map(|files| {
                                FileDialogAnswer::Paths(
                                    files
                                        .iter()
                                        .map(|p| p.to_string_lossy().into_owned())
                                        .collect(),
                                )
                            })
                        } else {
                            dialog.pick_file().map(|p| {
                                FileDialogAnswer::Paths(vec![p.to_string_lossy().into_owned()])
                            })
                        }
                    }
                    FileDialogRequest::Folder { initial_dir } => {
                        if let Some(dir) = initial_dir {
                            dialog = dialog.set_directory(dir);
                        }
                        dialog
                            .pick_folder()
                            .map(|p| FileDialogAnswer::FolderTo(p.to_string_lossy().into_owned()))
                    }
                    FileDialogRequest::Save { suggested } => {
                        for (name, exts) in save_filters(&suggested) {
                            dialog = dialog.add_filter(name, &exts);
                        }
                        if let Some(name) = Path::new(&suggested).file_name() {
                            dialog = dialog.set_file_name(name.to_string_lossy());
                        }
                        if let Some(dir) = Path::new(&suggested)
                            .parent()
                            .filter(|p| !p.as_os_str().is_empty())
                        {
                            dialog = dialog.set_directory(dir);
                        }
                        dialog
                            .save_file()
                            .map(|p| FileDialogAnswer::SaveTo(p.to_string_lossy().into_owned()))
                    }
                };
                reply.send(answer);
            });
        if let Err(e) = spawned {
            log::error!("couldn't wait for the file dialog: {e}");
        }
    }

    /// The services the Photo editor runs with. Not ported yet: screen colour picking, Caps Lock,
    /// drop cursor position, custom title bar frame, caption style, system theme, native menu,
    /// tablet input, automation workspace, Photoshop shortcut import.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn services() -> Services {
        use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
        let clip: Rc<RefCell<Option<arboard::Clipboard>>> = Rc::default();
        let prefs = prefs_enabled();
        let mut services = Services {
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
            file_dialog: Some(Box::new(show_file_dialog)),
            write: Some(Box::new(|path: &str, bytes: &[u8]| {
                write_atomic(Path::new(path), bytes)
            })),
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
            open_url: Some(Box::new(open_url)),
            clipboard_set_image: Some({
                let clip = clip.clone();
                Box::new(move |w: u32, h: u32, px: &[u8]| {
                    let mut slot = clip
                        .try_borrow_mut()
                        .map_err(|_| "clipboard is busy".to_string())?;
                    let cb = match slot.as_mut() {
                        Some(c) => c,
                        None => slot.insert(arboard::Clipboard::new().map_err(|e| e.to_string())?),
                    };
                    cb.set_image(arboard::ImageData {
                        width: w as usize,
                        height: h as usize,
                        bytes: std::borrow::Cow::Borrowed(px),
                    })
                    .map_err(|e| e.to_string())
                })
            }),
            clipboard_get_image: Some(Box::new(move || {
                let mut slot = clip.try_borrow_mut().ok()?;
                let cb = match slot.as_mut() {
                    Some(c) => c,
                    None => slot.insert(arboard::Clipboard::new().ok()?),
                };
                cb.get_image()
                    .ok()
                    .map(|img| (img.width as u32, img.height as u32, img.bytes.into_owned()))
            })),
            load_prefs: prefs.then(|| {
                Box::new(|| std::fs::read_to_string(prefs_file()?).ok())
                    as photocraft_ui_egui::LoadTextFn
            }),
            save_prefs: prefs.then(|| {
                Box::new(|text: &str| {
                    write_atomic(&prefs_file().ok_or("no config directory")?, text.as_bytes())
                }) as photocraft_ui_egui::SaveTextFn
            }),
            append_text: Some(Box::new(|path: &str, text: &str| {
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|e| e.to_string())?;
                f.write_all(text.as_bytes()).map_err(|e| e.to_string())
            })),
            ..recovery_services(recovery_dir())
        };
        // Brush presets load in the background; the app attaches them when they arrive.
        services.preset_store = presets_dir().map(photocraft_engine::preset_store::open_dir_async);
        services
    }
}

// ---------------------------------------------------------------------------------------------
// Vector (vectorcraft)
// ---------------------------------------------------------------------------------------------

pub mod vector {
    use super::*;
    use vectorcraft_ui_egui::{FilePick, Services, VectorcraftApp};

    pub fn prefs_path() -> Option<PathBuf> {
        editor_dir(Editor::Vector).map(|d| d.join("ui.json"))
    }

    fn sibling(name: &str) -> Option<String> {
        editor_dir(Editor::Vector).map(|d| d.join(name).to_string_lossy().into_owned())
    }

    fn write_file(path: &str, bytes: &[u8]) -> Result<(), String> {
        vectorcraft_format::write_atomic(Path::new(path), bytes).map_err(|e| e.to_string())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn dialog_for(pick: &FilePick) -> rfd::FileDialog {
        let mut d = rfd::FileDialog::new();
        for (name, exts) in &pick.filters {
            d = d.add_filter(*name, exts);
        }
        if let Some(folder) = &pick.folder {
            let _ = std::fs::create_dir_all(folder);
            d = d.set_directory(folder);
        }
        if !pick.name.is_empty() {
            d = d.set_file_name(&pick.name);
        }
        d
    }

    fn lossy(p: PathBuf) -> String {
        p.to_string_lossy().into_owned()
    }

    /// The services the Vector editor runs with. Dialogs are shown in line (the Linux
    /// off-thread picker, macOS sheets, the system clipboard formats, Show in Folder, Edit
    /// Original and printing are not ported yet).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn services() -> Services {
        Services {
            pick_open: Some(Box::new(|pick: &FilePick| {
                dialog_for(pick).pick_file().map(lossy)
            })),
            pick_open_multi: Some(Box::new(|| {
                rfd::FileDialog::new()
                    .pick_files()
                    .unwrap_or_default()
                    .into_iter()
                    .map(lossy)
                    .collect()
            })),
            pick_save: Some(Box::new(|pick: &FilePick| {
                dialog_for(pick).save_file().map(lossy)
            })),
            pick_folder: Some(Box::new(|| rfd::FileDialog::new().pick_folder().map(lossy))),
            read: Some(Box::new(read_bytes)),
            write: Some(Box::new(write_file)),
            write_shared: Some(Arc::new(write_file)),
            stream_shared: Some(Arc::new(
                |p: &str, fill: &mut dyn FnMut(&mut dyn std::io::Write) -> std::io::Result<()>| {
                    vectorcraft_engine::cmd::fileio::write_file_with(p, fill)
                        .map_err(|e| e.to_string())
                },
            )),
            open_url: Some(Box::new(|url: &str| {
                let _ = open_url(url);
            })),
            ..Default::default()
        }
    }

    /// Have the font scans read the editor's own Fonts folder, which `text.addFontFiles`
    /// copies font files into.
    pub fn install_fonts() {
        if let Some(dir) = editor_dir(Editor::Vector) {
            vectorcraft_text::set_app_font_dir(dir.join("Fonts"));
        }
    }

    /// User swatch / graphic style / object libraries and Data Recovery next to the preferences.
    pub fn attach_dirs(app: &mut VectorcraftApp) {
        app.session
            .swatch_libraries
            .set_user_dir(sibling("Swatches"));
        app.session
            .style_libraries
            .set_user_dir(sibling("Graphic Styles"));
        if prefs_enabled() {
            app.session.libraries.set_dir(sibling("Libraries"));
            app.session
                .recovery
                .set_default_folder(sibling("Data Recovery"));
        }
    }

    pub fn load_prefs(app: &mut VectorcraftApp) {
        if !prefs_enabled() {
            return;
        }
        if let Some(bytes) = prefs_path().and_then(|p| std::fs::read(p).ok())
            && let Ok(ui) = serde_json::from_slice::<vectorcraft_ui_egui::UiState>(&bytes)
        {
            app.ui = ui.sanitized();
        }
        vectorcraft_ui_egui::prefs_dialog::restore(app);
    }

    pub fn save_prefs(app: &VectorcraftApp) {
        if !prefs_enabled() {
            return;
        }
        if let Some(p) = prefs_path() {
            let mut ui = app.ui.clone();
            ui.engine_prefs = app.session.prefs.to_json();
            if let Ok(bytes) = serde_json::to_vec_pretty(&ui) {
                let _ = vectorcraft_format::write_atomic(&p, &bytes);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Layout (designcraft)
// ---------------------------------------------------------------------------------------------

pub mod layout {
    use super::*;
    use designcraft_ui_egui::{DesignApp, Services};

    pub fn prefs_path() -> Option<PathBuf> {
        editor_dir(Editor::Layout).map(|d| d.join("ui.json"))
    }

    /// This run's folder in the recovery folder; `None` when the run cannot be isolated.
    pub fn recovery_dir() -> Option<PathBuf> {
        let root = editor_dir(Editor::Layout)?.join("Recovery");
        match designcraft_engine::recovery::run_dir(&root) {
            Ok(dir) => Some(dir),
            Err(e) => {
                log::warn!("recovery is unavailable because this run cannot be isolated: {e}");
                None
            }
        }
    }

    struct OpenFilter {
        name: &'static str,
        extensions: &'static [&'static str],
    }

    fn open_filters(purpose: &str) -> &'static [OpenFilter] {
        match purpose {
            "swatches" => &[OpenFilter {
                name: "Swatch Exchange (ASE)",
                extensions: &["ase"],
            }],
            "script" => &[OpenFilter {
                name: "Script",
                extensions: &["dcscript", "txt", "json"],
            }],
            "icc" => &[OpenFilter {
                name: "ICC profile",
                extensions: &["icc", "icm"],
            }],
            "book" => &[OpenFilter {
                name: "Book",
                extensions: &["dcbook"],
            }],
            "xml" => &[OpenFilter {
                name: "XML",
                extensions: &["xml"],
            }],
            "library" => &[OpenFilter {
                name: "Object Library",
                extensions: &["dclib"],
            }],
            "dataMerge" => &[OpenFilter {
                name: "Data source (CSV, TSV, text, Excel)",
                extensions: &["csv", "tsv", "tab", "txt", "xlsx"],
            }],
            "place" => &[
                OpenFilter {
                    name: "Graphics and text",
                    extensions: &[
                        "png",
                        "jpg",
                        "jpeg",
                        "gif",
                        "webp",
                        "tif",
                        "tiff",
                        "bmp",
                        "psd",
                        "svg",
                        "pdf",
                        "ai",
                        "eps",
                        "txt",
                        "docx",
                        "rtf",
                        "md",
                        "xlsx",
                        "idml",
                        "designcraft",
                        "mp4",
                        "m4v",
                        "mov",
                        "webm",
                        "mp3",
                        "m4a",
                        "wav",
                        "ogg",
                    ],
                },
                OpenFilter {
                    name: "Graphics",
                    extensions: &[
                        "png", "jpg", "jpeg", "gif", "webp", "tif", "tiff", "bmp", "psd", "svg",
                        "pdf", "ai", "eps",
                    ],
                },
                OpenFilter {
                    name: "Text (Word, RTF, plain, Excel)",
                    extensions: &["docx", "rtf", "txt", "md", "xlsx"],
                },
                OpenFilter {
                    name: "Video and sound",
                    extensions: &["mp4", "m4v", "mov", "webm", "mp3", "m4a", "wav", "ogg"],
                },
            ],
            _ => &[
                OpenFilter {
                    name: "Redrob Canvas layout or IDML",
                    extensions: &["designcraft", "idml"],
                },
                OpenFilter {
                    name: "Redrob Canvas layout",
                    extensions: &["designcraft"],
                },
                OpenFilter {
                    name: "InDesign Markup (IDML)",
                    extensions: &["idml"],
                },
            ],
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn services() -> Services {
        Services {
            pick_open: Some(Box::new(|purpose: &str| {
                let mut dialog = rfd::FileDialog::new();
                for filter in open_filters(purpose) {
                    dialog = dialog.add_filter(filter.name, filter.extensions);
                }
                dialog.pick_file().map(|p| p.to_string_lossy().to_string())
            })),
            pick_save: Some(Box::new(|name: &str| {
                rfd::FileDialog::new()
                    .set_file_name(name)
                    .save_file()
                    .map(|p| p.to_string_lossy().to_string())
            })),
            read: Some(Box::new(read_bytes)),
            write: Some(Box::new(|p: &str, b: &[u8]| {
                designcraft_engine::atomic::write_atomic(Path::new(p), b).map_err(|e| e.to_string())
            })),
            clipboard_text: Some(Box::new(|| {
                arboard::Clipboard::new()
                    .and_then(|mut c| c.get_text())
                    .ok()
            })),
            ..Default::default()
        }
    }

    pub fn load_prefs(app: &mut DesignApp) {
        if !prefs_enabled() {
            return;
        }
        if let Some(p) = prefs_path()
            && let Ok(bytes) = std::fs::read(&p)
            && let Ok(ui) = designcraft_ui_egui::UiState::from_preferences_json(&bytes)
        {
            app.ui = ui;
        }
        if let Some(p) = prefs_path().map(|p| p.with_file_name("prefs.json"))
            && let Ok(bytes) = std::fs::read(&p)
            && let Ok(prefs) = serde_json::from_slice::<designcraft_engine::Prefs>(&bytes)
        {
            app.session.prefs = prefs;
        }
    }

    pub fn save_prefs(app: &DesignApp) {
        if !prefs_enabled() {
            return;
        }
        if let Some(p) = prefs_path() {
            if let Ok(bytes) = serde_json::to_vec_pretty(&app.ui) {
                let _ = std::fs::write(&p, bytes);
            }
            if let Ok(bytes) = serde_json::to_vec_pretty(&app.session.prefs) {
                let _ = std::fs::write(p.with_file_name("prefs.json"), bytes);
            }
        }
    }

    /// Orderly OS termination can bypass egui's cancelable close event: finish a complete quit
    /// plan, else preserve every live document for the next start (upstream
    /// `preserve_documents_on_exit`).
    pub fn preserve_documents_on_exit(app: &mut DesignApp) -> Result<(), String> {
        app.finish_confirmed_quit()?;
        if app.session.documents().is_empty() {
            return Ok(());
        }
        if app.session.recovery_dir.is_none() {
            return if app.session.documents().iter().any(|d| d.is_dirty()) {
                Err(
                    "Recovery is unavailable; unsaved documents could not be preserved on exit"
                        .into(),
                )
            } else {
                Ok(())
            };
        }
        app.run("file.recovery.save", serde_json::json!({}))
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(key: &str) -> Option<OsString> {
        match key {
            "HOME" => Some("/user".into()),
            "APPDATA" => Some("/roaming".into()),
            "XDG_CONFIG_HOME" => Some("/xdg-config".into()),
            _ => None,
        }
    }

    #[test]
    fn config_dir_follows_each_platform_convention() {
        assert_eq!(
            config_dir_with("macos", environment),
            Some("/user/Library/Application Support/Redrob Canvas".into())
        );
        assert_eq!(
            config_dir_with("windows", environment),
            Some("/roaming/Redrob Canvas".into())
        );
        assert_eq!(
            config_dir_with("linux", environment),
            Some("/xdg-config/redrob-canvas".into())
        );
        assert_eq!(
            config_dir_with("linux", |key| if key == "HOME" {
                environment(key)
            } else {
                None
            }),
            Some("/user/.config/redrob-canvas".into())
        );
        assert_eq!(config_dir_with("linux", |_| None), None);
    }

    #[test]
    fn config_dir_override_wins_and_an_empty_override_disables_it() {
        for os in ["macos", "windows", "linux"] {
            let dir = config_dir_with(os, |key| {
                if key == CONFIG_DIR_ENV {
                    Some("/isolated".into())
                } else {
                    environment(key)
                }
            });
            assert_eq!(dir, Some(PathBuf::from("/isolated")), "{os}");
            let none = config_dir_with(os, |key| {
                if key == CONFIG_DIR_ENV {
                    Some(OsString::new())
                } else {
                    environment(key)
                }
            });
            assert_eq!(none, None, "{os}");
        }
    }

    #[test]
    fn each_editor_has_its_own_folder() {
        let folders: std::collections::BTreeSet<_> =
            [Editor::Photo, Editor::Vector, Editor::Layout]
                .iter()
                .map(|e| e.folder())
                .collect();
        assert_eq!(folders.len(), 3);
    }
}
