// SPDX-License-Identifier: MIT OR Apache-2.0

//! Finder's double-click and "Open With" for the Redrob Canvas window on macOS.
//!
//! Finder passes no file arguments: it sends the app an open-documents Apple Event, which AppKit
//! turns into `application:openURLs:` on the application delegate. That delegate is winit's own
//! `WinitApplicationDelegate`, which does not implement the method, so without this crate the
//! event is dropped and the app opens empty. [`install`] adds the method; on other systems it does
//! nothing (Windows and Linux pass the file as an argument).

use std::path::PathBuf;

#[cfg(target_os = "macos")]
mod macos;

/// Where opened files go. Called on the main thread with the paths of one open-documents event.
pub type Handler = fn(Vec<PathBuf>);

/// Call once, on the main thread, after AppKit's shared application exists or before the event
/// loop is built (eframe's `event_loop_builder` hook) and before it runs, so the files of the
/// launch itself are not missed. Each later open-documents event reaches `handler` too.
pub fn install(handler: Handler) {
    #[cfg(target_os = "macos")]
    macos::install(handler);
    #[cfg(not(target_os = "macos"))]
    let _ = handler;
}
