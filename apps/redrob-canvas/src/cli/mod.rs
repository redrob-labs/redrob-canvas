//! The command line: `redrob-canvas photo|vector|layout ARGS` runs one editor headless (convert,
//! inspect, batch, MCP server) instead of opening the window.

mod layout;
mod photo;
mod vector;

use std::process::ExitCode;

/// The editors the command line serves, by their first argument.
const EDITORS: [&str; 3] = ["photo", "vector", "layout"];

/// Runs the command line when the first argument names an editor; `None` opens the window.
pub fn dispatch() -> Option<ExitCode> {
    let mut args = std::env::args_os().skip(1);
    let editor = args.next()?.into_string().ok()?;
    if !EDITORS.contains(&editor.as_str()) {
        return None;
    }
    attach_console();
    let rest: Vec<std::ffi::OsString> = args.collect();
    Some(match editor.as_str() {
        "photo" => {
            let code = photo::run_os(&rest, &mut std::io::stdout(), &mut std::io::stderr());
            ExitCode::from(u8::try_from(code).unwrap_or(1))
        }
        editor => {
            let mut strings = Vec::with_capacity(rest.len());
            for (i, a) in rest.into_iter().enumerate() {
                match a.into_string() {
                    Ok(s) => strings.push(s),
                    Err(raw) => {
                        eprintln!(
                            "redrob-canvas {editor}: argument {} is not valid Unicode: `{}`",
                            i + 1,
                            raw.to_string_lossy()
                        );
                        return Some(ExitCode::from(2));
                    }
                }
            }
            if editor == "vector" {
                vector::main(strings)
            } else {
                layout::main(strings)
            }
        }
    })
}

/// The release build on Windows is a GUI program with no console of its own, so its output would
/// go nowhere; the command line writes to the console it was started from instead.
#[cfg(windows)]
fn attach_console() {
    let _ = winsafe::AttachConsole(winsafe::PidParent::Parent);
}

#[cfg(not(windows))]
fn attach_console() {}
