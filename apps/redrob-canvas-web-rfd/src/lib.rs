// SPDX-License-Identifier: MIT OR Apache-2.0

//! `rfd` as the Redrob Canvas web shell sees it.
//!
//! The shell's `open_dialog` (apps/redrob-canvas/src/shell.rs) builds a synchronous
//! `rfd::FileDialog` and calls `pick_files()`. rfd does not build that type for wasm32 (there is
//! no blocking picker in a browser), so apps/redrob-canvas-web depends on THIS crate under the
//! name `rfd`: the real crate re-exported whole (`AsyncFileDialog`, `FileHandle`, which the web
//! services use), plus a [`FileDialog`] with the same builder surface whose pickers answer
//! `None` and log that the browser's picker is the editor's own File > Open. That keeps shell.rs
//! one source for both builds; nothing here is used on the desktop.
//!
//! On a non-wasm target the crate is the stand-in alone, so the workspace's host builds compile
//! it without pulling anything.

#![deny(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

#[cfg(target_arch = "wasm32")]
pub use rfd::*;

/// Stands in for rfd's synchronous `FileDialog` on wasm32: a builder that remembers its filters
/// and title and whose pickers return `None`.
#[derive(Clone, Debug, Default)]
pub struct FileDialog {
    filters: Vec<(String, Vec<String>)>,
    title: Option<String>,
    directory: Option<PathBuf>,
    file_name: Option<String>,
}

impl FileDialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_filter(mut self, name: impl Into<String>, extensions: &[impl ToString]) -> Self {
        self.filters.push((
            name.into(),
            extensions.iter().map(ToString::to_string).collect(),
        ));
        self
    }

    pub fn set_directory<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.directory = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn set_file_name(mut self, file_name: impl Into<String>) -> Self {
        self.file_name = Some(file_name.into());
        self
    }

    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// The filters added so far, as (name, extensions).
    pub fn filters(&self) -> &[(String, Vec<String>)] {
        &self.filters
    }

    fn unavailable(&self, what: &str) {
        log::info!(
            "{what}: no blocking file picker in the browser{}; use the editor's own File > Open",
            self.title
                .as_deref()
                .map(|t| format!(" ({t})"))
                .unwrap_or_default()
        );
    }

    pub fn pick_file(self) -> Option<PathBuf> {
        self.unavailable("pick_file");
        None
    }

    pub fn pick_files(self) -> Option<Vec<PathBuf>> {
        self.unavailable("pick_files");
        None
    }

    pub fn pick_folder(self) -> Option<PathBuf> {
        self.unavailable("pick_folder");
        None
    }

    pub fn save_file(self) -> Option<PathBuf> {
        self.unavailable("save_file");
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stand_in_keeps_the_builder_shape_and_picks_nothing() {
        let dialog = FileDialog::new()
            .set_title("Open in Redrob Canvas")
            .add_filter("All documents", &["pcraft", "svg"])
            .add_filter("Photo", &["png"]);
        assert_eq!(dialog.filters().len(), 2);
        assert_eq!(dialog.filters()[0].1, vec!["pcraft", "svg"]);
        assert_eq!(dialog.clone().pick_files(), None);
        assert_eq!(dialog.clone().pick_file(), None);
        assert_eq!(dialog.clone().pick_folder(), None);
        assert_eq!(dialog.save_file(), None);
    }
}
