// SPDX-License-Identifier: MIT OR Apache-2.0

//! The browser shell: the eframe web runner around [`ShellApp`], dropped files, and the
//! unsaved-changes guard on closing the tab.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wasm_bindgen::JsCast as _;

use crate::routing;
use crate::services;
use crate::shell::{Mode, ShellApp};

const CANVAS_ID: &str = "redrob_canvas";
const LOADING_ID: &str = "redrob_canvas_loading";

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            log::error!("no document");
            return;
        };
        let Some(canvas) = document
            .get_element_by_id(CANVAS_ID)
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        else {
            log::error!("missing <canvas id=\"{CANVAS_ID}\">");
            return;
        };
        let query = query();
        let mut options = eframe::WebOptions::default();
        // The Photo editor's GPU canvas asks for the adapter's own limits (large textures).
        photocraft_ui_egui::gpu_canvas::use_adapter_limits(&mut options.wgpu_options.wgpu_setup);
        if query.contains("webgl")
            && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) =
                &mut options.wgpu_options.wgpu_setup
        {
            create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
        }
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    if let Some(rs) = &cc.wgpu_render_state {
                        log::info!(
                            "redrob-canvas-web: wgpu backend {:?}",
                            rs.adapter.get_info().backend
                        );
                    }
                    let mut shell = ShellApp::new(cc.wgpu_render_state.clone());
                    shell.set_mode(mode_from_query(&query).unwrap_or_default());
                    let unsaved = Arc::new(AtomicBool::new(false));
                    guard_unload(unsaved.clone());
                    Ok(Box::new(WebShell { shell, unsaved }))
                }),
            )
            .await;
        if let Some(el) = document.get_element_by_id(LOADING_ID) {
            match result {
                Ok(()) => el.remove(),
                Err(e) => el.set_inner_html(&format!(
                    "<p>Redrob Canvas failed to start: {e:?}</p>\
                     <p>A browser with WebGPU or WebGL2 is required.</p>"
                )),
            }
        }
    });
}

/// `?mode=photo|vector|layout`, like the desktop's `--mode`.
fn mode_from_query(query: &str) -> Option<Mode> {
    let value = query
        .trim_start_matches('?')
        .split('&')
        .find_map(|pair| pair.strip_prefix("mode="))?;
    match value.to_ascii_lowercase().as_str() {
        "photo" => Some(Mode::Photo),
        "vector" => Some(Mode::Vector),
        "layout" => Some(Mode::Layout),
        _ => None,
    }
}

fn query() -> String {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default()
}

/// Closing or reloading the tab while a document has unsaved changes asks first, as closing the
/// desktop window does: the browser shows its own "Leave site?" prompt. `unsaved` is refreshed
/// every frame by [`WebShell`].
fn guard_unload(unsaved: Arc<AtomicBool>) {
    use wasm_bindgen::closure::Closure;
    let Some(window) = web_sys::window() else {
        return;
    };
    let cb = Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(
        move |e: web_sys::BeforeUnloadEvent| {
            if unsaved.load(Ordering::Relaxed) {
                e.prevent_default();
                // Older browsers show the prompt only when a return value is set.
                e.set_return_value("");
            }
        },
    );
    if window
        .add_event_listener_with_callback("beforeunload", cb.as_ref().unchecked_ref())
        .is_ok()
    {
        cb.forget();
    }
}

/// The eframe app: the shell, plus what only a browser needs around it.
struct WebShell {
    shell: ShellApp,
    unsaved: Arc<AtomicBool>,
}

impl WebShell {
    /// Files dropped on the window carry contents, not paths, in a browser, and the shell's own
    /// drop routing opens by path. Taken here before the shell sees them: each is read
    /// asynchronously and delivered to the editor its extension belongs to, which the shell
    /// switches to.
    fn take_drops(&mut self, raw: &mut egui::RawInput) {
        let dropped = std::mem::take(&mut raw.dropped_files);
        for file in dropped {
            let name = file
                .path()
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "dropped".into());
            let Some(route) = routing::route(Path::new(&name)) else {
                log::warn!("{name}: no editor opens this kind of file");
                continue;
            };
            let mode = route.mode;
            self.shell.set_mode(mode);
            let layout_request =
                (mode == Mode::Layout).then(|| self.shell.layout_app().import_request("drop"));
            wasm_bindgen_futures::spawn_local(async move {
                match file.bytes_async().await {
                    Ok(bytes) => match layout_request {
                        Some(request) => services::deliver_layout(request, name, bytes),
                        None => services::deliver(mode, name, bytes),
                    },
                    Err(e) => log::error!("couldn't read dropped file {name}: {e}"),
                }
            });
        }
    }
}

impl eframe::App for WebShell {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        eframe::App::logic(&mut self.shell, ctx, frame);
        let dirty = self.shell.dirty_counts().iter().any(|(_, n)| *n > 0);
        self.unsaved.store(dirty, Ordering::Relaxed);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.take_drops(raw);
        eframe::App::raw_input_hook(&mut self.shell, ctx, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        eframe::App::ui(&mut self.shell, ui, frame);
    }

    fn on_exit(&mut self) {
        eframe::App::on_exit(&mut self.shell);
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_query_flag_is_the_desktop_mode_argument() {
        assert_eq!(mode_from_query("?mode=vector"), Some(Mode::Vector));
        assert_eq!(mode_from_query("?webgl&mode=Layout"), Some(Mode::Layout));
        assert_eq!(mode_from_query("?mode=3d"), None);
        assert_eq!(mode_from_query(""), None);
    }
}
