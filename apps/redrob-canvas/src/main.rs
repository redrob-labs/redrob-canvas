//! Redrob Canvas desktop app: photo, vector and layout editing in one window.
//!
//! Usage: `redrob-canvas [--mode photo|vector|layout] [--version]`
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod ai;
mod handoff;
mod i18n;
mod routing;
mod services;
mod shell;
mod trace;
mod update;

use shell::{Mode, ShellApp};

/// The main window: 1440 × 900 (eframe shrinks it to the monitor), at least 760 × 480.
fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(services::APP_ID)
            .with_title(services::APP_NAME)
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([760.0, 480.0])
            .with_drag_and_drop(true),
        centered: true,
        // eframe saves the window geometry and egui panel sizes beside the preferences.
        persistence_path: services::persistence_path(),
        ..Default::default()
    }
}

/// `--mode photo|vector|layout`: the editor the window opens on.
pub fn parse_mode(value: &str) -> Option<Mode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "photo" => Some(Mode::Photo),
        "vector" => Some(Mode::Vector),
        "layout" => Some(Mode::Layout),
        _ => None,
    }
}

fn main() -> eframe::Result {
    trace::init();
    let mut mode = Mode::Photo;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--mode" => match args.next().as_deref().and_then(parse_mode) {
                Some(m) => mode = m,
                None => {
                    eprintln!("redrob-canvas: --mode expects photo, vector or layout");
                    std::process::exit(2);
                }
            },
            "--version" => {
                println!("redrob-canvas {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => eprintln!("redrob-canvas: ignoring argument {other:?}"),
        }
    }
    // TODO(control server): no control / MCP server yet; see `services` for the upstream paths.
    let mut options = native_options();
    // Nothing draws or dispatches indirectly, so wgpu's indirect-call validation only costs a
    // compute shader at start-up that some drivers cannot compile (as the upstream apps found).
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        create.instance_descriptor.flags = create
            .instance_descriptor
            .flags
            .difference(eframe::wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL)
            .with_env();
    }
    eframe::run_native(
        services::APP_NAME,
        options,
        Box::new(move |cc| {
            if let Some(rs) = &cc.wgpu_render_state {
                let info = rs.adapter.get_info();
                log::info!(
                    "graphics: {} on {:?} ({:?}; driver {} {})",
                    info.name,
                    info.backend,
                    info.device_type,
                    info.driver,
                    info.driver_info
                );
            }
            let mut app = ShellApp::new(cc.wgpu_render_state.clone());
            app.setup_context(&cc.egui_ctx);
            app.set_mode(mode);
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_opens_at_its_default_size_with_a_floor() {
        let o = native_options();
        assert_eq!(o.viewport.inner_size, Some(egui::vec2(1440.0, 900.0)));
        assert_eq!(o.viewport.min_inner_size, Some(egui::vec2(760.0, 480.0)));
        assert_eq!(o.viewport.title.as_deref(), Some("Redrob Canvas"));
        assert_eq!(o.viewport.app_id.as_deref(), Some("ai.redrob.canvas"));
        assert!(o.centered);
    }

    #[test]
    fn mode_arguments_parse_case_insensitively() {
        assert_eq!(parse_mode("photo"), Some(Mode::Photo));
        assert_eq!(parse_mode(" Vector "), Some(Mode::Vector));
        assert_eq!(parse_mode("LAYOUT"), Some(Mode::Layout));
        assert_eq!(parse_mode("3d"), None);
    }
}
