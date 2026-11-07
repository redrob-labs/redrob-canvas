// SPDX-License-Identifier: MIT OR Apache-2.0

//! Files the operating system asks the window to open: a bare path on the command line (Explorer's
//! `"redrob-canvas.exe" "%1"`, a Linux file manager) and, on macOS, the open-documents event
//! (the redrob-open-documents crate). They queue here from any thread and before the window exists; the shell takes
//! them each frame and opens them like the Open dialog's choices (`ShellApp::open_paths`).

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static QUEUE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

/// Queues `paths` and wakes the window when it is up.
pub fn push(paths: impl IntoIterator<Item = PathBuf>) {
    let mut queue = QUEUE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = queue.len();
    queue.extend(paths);
    let added = queue.len() > before;
    drop(queue);
    if added && let Some(ctx) = CONTEXT.get() {
        ctx.request_repaint();
    }
}

/// [`push`] as a plain function, for the macOS open-documents handler.
#[cfg(target_os = "macos")]
pub fn push_vec(paths: Vec<PathBuf>) {
    push(paths);
}

/// Everything queued so far.
pub fn take() -> Vec<PathBuf> {
    std::mem::take(
        &mut *QUEUE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

/// The window's context, so a later [`push`] wakes an idle window.
pub fn set_context(ctx: &egui::Context) {
    let _ = CONTEXT.set(ctx.clone());
}
