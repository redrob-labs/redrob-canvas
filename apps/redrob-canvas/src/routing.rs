// SPDX-License-Identifier: MIT OR Apache-2.0

//! Which editor a file opens in. The shell's Open and a drop onto the window route a path by its
//! extension to a mode and hand it to that editor's own open command (the import itself stays
//! upstream's). A PDF is the one ambiguous case: it opens editable in Vector, and the status line
//! offers to place it in Layout instead.

use std::path::Path;

use crate::shell::Mode;

/// Extensions that open in Photo mode: the native document and the project's own, layered
/// interchange, plain rasters, HDR, camera raw. The engine's own batch list is
/// crafts/photocraft/crates/engine/src/file_cmds.rs L121 (`OPENABLE`), without SVG (editable in
/// Vector) and the Affinity family it takes as previews.
pub const PHOTO_EXTENSIONS: &[&str] = &[
    "psd", "psb", "pcraft", "ora", "kra", "krz", "pdn", "tif", "tiff", "png", "jpg", "jpeg",
    "webp", "avif", "heic", "heif", "hif", "gif", "bmp", "tga", "exr", "hdr", "qoi", "ico", "pnm",
    "ppm", "pgm", "dpx", "fits", "fit", "fts", "dng", "cr2", "cr3", "nef", "nrw", "arw", "pef",
    "raf", "orf", "rw2", "afphoto",
];

/// Extensions that open in Vector mode: SVG, Illustrator, PostScript, PDF, CAD and the Windows
/// metafiles, plus the native document (`redrob_vector_ui_egui::io::open_path`,
/// crafts/vectorcraft/crates/ui-egui/src/io.rs L218).
pub const VECTOR_EXTENSIONS: &[&str] = &[
    "svg",
    "svgz",
    "ai",
    "eps",
    "pdf",
    "dxf",
    "emf",
    "wmf",
    "vectorcraft",
    "afdesign",
];

/// Extensions that open in Layout mode. The layouts themselves open as documents
/// (`redrob_layout_ui_egui::opens_as_document`, crafts/designcraft/crates/ui-egui/src/lib.rs
/// L1452); word-processing and spreadsheet files are placed into frames by `file.place`
/// (`DesignApp::open_dropped`, L1140).
pub const LAYOUT_EXTENSIONS: &[&str] = &[
    "idml",
    "designcraft",
    "indd",
    "indt",
    "docx",
    "rtf",
    "md",
    "xlsx",
    "afpub",
];

/// Where a file goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    /// The mode the file opens in.
    pub mode: Mode,
    /// A second mode that can take the file too, offered in the status line ("Open in Layout
    /// instead" for a PDF).
    pub alternative: Option<Mode>,
}

impl Route {
    const fn only(mode: Mode) -> Self {
        Self {
            mode,
            alternative: None,
        }
    }
}

/// `path`'s extension, lower-case and without the dot (empty when it has none).
pub fn extension_of(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// The mode a file with extension `ext` (lower-case, no dot) opens in; `None` for a file no
/// editor claims.
pub fn route_extension(ext: &str) -> Option<Route> {
    if ext == "pdf" {
        return Some(Route {
            mode: Mode::Vector,
            alternative: Some(Mode::Layout),
        });
    }
    if PHOTO_EXTENSIONS.contains(&ext) {
        return Some(Route::only(Mode::Photo));
    }
    if VECTOR_EXTENSIONS.contains(&ext) {
        return Some(Route::only(Mode::Vector));
    }
    if LAYOUT_EXTENSIONS.contains(&ext) {
        return Some(Route::only(Mode::Layout));
    }
    None
}

/// [`route_extension`] for a path.
pub fn route(path: &Path) -> Option<Route> {
    route_extension(&extension_of(path))
}

/// The extensions a mode opens, for the Open dialog's filters.
pub fn extensions_for(mode: Mode) -> &'static [&'static str] {
    match mode {
        Mode::Photo => PHOTO_EXTENSIONS,
        Mode::Vector => VECTOR_EXTENSIONS,
        Mode::Layout => LAYOUT_EXTENSIONS,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn extensions_route_to_their_editor() {
        let cases = [
            ("shot.PSD", Mode::Photo),
            ("scan.tiff", Mode::Photo),
            ("raw.cr3", Mode::Photo),
            ("photo.jpeg", Mode::Photo),
            ("logo.svg", Mode::Vector),
            ("art.ai", Mode::Vector),
            ("plan.dxf", Mode::Vector),
            ("clip.wmf", Mode::Vector),
            ("doc.vectorcraft", Mode::Vector),
            ("book.idml", Mode::Layout),
            ("poster.designcraft", Mode::Layout),
            ("text.docx", Mode::Layout),
            ("notes.md", Mode::Layout),
            ("table.xlsx", Mode::Layout),
        ];
        for (name, mode) in cases {
            let r = route(Path::new(name)).unwrap_or_else(|| panic!("{name} has no route"));
            assert_eq!(r.mode, mode, "{name}");
            assert_eq!(r.alternative, None, "{name}");
        }
    }

    #[test]
    fn pdf_opens_in_vector_and_offers_layout() {
        let r = route(Path::new("/tmp/brochure.pdf")).unwrap();
        assert_eq!(r.mode, Mode::Vector);
        assert_eq!(r.alternative, Some(Mode::Layout));
    }

    #[test]
    fn unknown_and_missing_extensions_have_no_route() {
        assert_eq!(route(Path::new("archive.zip")), None);
        assert_eq!(route(Path::new("Makefile")), None);
        assert_eq!(route(Path::new("")), None);
        assert_eq!(extension_of(Path::new("a/b.TXT")), "txt");
        assert_eq!(extension_of(Path::new("a/b")), "");
    }

    #[test]
    fn no_extension_is_claimed_by_two_modes() {
        let mut all: Vec<&str> = Vec::new();
        for mode in Mode::ALL {
            for ext in extensions_for(mode) {
                assert!(!all.contains(ext), "{ext} listed twice");
                assert_eq!(*ext, ext.to_ascii_lowercase(), "{ext} must be lower-case");
                all.push(ext);
            }
        }
        assert!(all.contains(&"pdf"));
    }
}
