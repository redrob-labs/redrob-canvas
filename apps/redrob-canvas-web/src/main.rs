// SPDX-License-Identifier: MIT OR Apache-2.0

//! Redrob Canvas in the browser.
//!
//! The same [`shell::ShellApp`] as the desktop app, run through eframe's web runner (wgpu:
//! WebGPU where available, WebGL2 otherwise). Build with `trunk build --release` from this
//! directory; `.github/workflows/release.yml`'s `web` job zips the result as
//! `redrob-canvas-web-<version>.zip`.
//!
//! How the shell gets here: its sources are compiled INTO this crate by `#[path]` (`shell`,
//! `handoff`, `routing`, and the AI layer's pure `bridge`), with three modules replaced by wasm
//! glue of this crate's own:
//! - `services` (src/services.rs): browser file pickers through `rfd::AsyncFileDialog`, saves
//!   and exports as downloads, preferences in `localStorage`; no config directory, no recovery
//!   folders.
//! - `ai` (src/ai/): the AI menu, dock toggle and dock, every tool disabled with "AI tools need
//!   the desktop app"; the IOPaint engine and the agent need tokio and a local process.
//! - `update` (src/update.rs): no in-app updater (a page is always the version its host serves).
//!
//! Differences from the desktop app, beyond those: no control / MCP server, File > Open is the
//! browser's picker and the bytes arrive asynchronously through each editor's inbox, dropped files
//! are read the same way, and the shell's own Open (Mode > Open…, Ctrl+Shift+O) says to use the
//! editor's File > Open.
//!
//! URL query flags: `?webgl` forces the WebGL2 backend instead of WebGPU.

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#![deny(clippy::unwrap_used, clippy::expect_used)]

#[cfg(target_arch = "wasm32")]
mod ai;
#[cfg(target_arch = "wasm32")]
#[path = "../../redrob-canvas/src/handoff.rs"]
mod handoff;
#[cfg(target_arch = "wasm32")]
#[path = "../../redrob-canvas/src/routing.rs"]
mod routing;
#[cfg(target_arch = "wasm32")]
mod services;
#[cfg(target_arch = "wasm32")]
#[path = "../../redrob-canvas/src/shell.rs"]
mod shell;
#[cfg(target_arch = "wasm32")]
mod update;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!(
        "redrob-canvas-web only runs in the browser: build it with `trunk build --release` in apps/redrob-canvas-web"
    );
}

#[cfg(test)]
mod tests {
    /// A WebAssembly module that never downloads (or doesn't match the page) runs no Rust, so the
    /// page itself replaces the endless "Loading" with what may have gone wrong and a way to
    /// retry, without a hand-written script.
    #[test]
    fn loading_page_offers_a_reload_when_startup_stalls() {
        let html = include_str!("../index.html");
        let (_, hint) = html
            .split_once("<p id=\"redrob_canvas_stalled\">")
            .unwrap_or_default();
        assert!(
            hint.contains("failed to") && hint.contains("<a href=\"\">Reload</a>"),
            "{hint}"
        );
        assert!(
            html.contains("animation: redrob-canvas-stalled"),
            "the hint appears after a delay"
        );
        assert!(
            !html.contains("<script"),
            "trunk injects the generated loader; the page has no script of its own"
        );
        assert!(html.contains("data-bin=\"redrob-canvas-web\""));
        assert!(html.contains("<canvas id=\"redrob_canvas\">"));
    }

    /// The page and the runner agree on the canvas and loading element ids.
    #[test]
    fn the_page_has_the_ids_the_runner_looks_up() {
        let html = include_str!("../index.html");
        let web = include_str!("web.rs");
        for id in ["redrob_canvas", "redrob_canvas_loading"] {
            assert!(html.contains(&format!("id=\"{id}\"")), "{id} in index.html");
            assert!(web.contains(&format!("\"{id}\"")), "{id} in web.rs");
        }
    }
}
