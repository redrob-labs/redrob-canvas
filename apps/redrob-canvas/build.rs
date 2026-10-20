// Windows only.
//
// 1. Windows gives the main thread a 1 MiB stack (Linux and macOS give 8 MiB). Building the
//    editor state on start-up needs more than 1 MiB, so the Windows release died at launch with
//    "thread 'main' has overflowed its stack" (0xC00000FD). Ask the linker for the same 8 MiB.
// 2. Embed the app icon in redrob-canvas.exe, so Explorer, the taskbar, the Start menu and
//    Alt-Tab show it. Without it the exe has Windows' blank default icon.
fn main() {
    const STACK: u32 = 8 * 1024 * 1024;
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../resources/icons/redrob-canvas.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => println!("cargo:rustc-link-arg-bins=/STACK:{STACK}"),
        Ok("gnu") => println!("cargo:rustc-link-arg-bins=-Wl,--stack,{STACK}"),
        _ => {}
    }
    icon::embed();
}

#[cfg(windows)]
mod icon {
    pub fn embed() {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../resources/icons/redrob-canvas.ico")
            .set("ProductName", "Redrob Canvas")
            .set("FileDescription", "Redrob Canvas")
            .set("OriginalFilename", "redrob-canvas.exe")
            .set("InternalName", "redrob-canvas");
        if let Err(e) = res.compile() {
            panic!("embedding the Windows icon failed: {e}");
        }
    }
}

/// Cross-compiling for Windows from another host: `winresource` is a Windows-only build-dependency.
#[cfg(not(windows))]
mod icon {
    pub fn embed() {
        println!("cargo:warning=redrob-canvas.exe built without its icon (non-Windows host)");
    }
}
