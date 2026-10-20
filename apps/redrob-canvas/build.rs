// Windows gives the main thread a 1 MiB stack (Linux and macOS give 8 MiB). Building the editor
// state on start-up needs more than 1 MiB, so the Windows release died at launch with
// "thread 'main' has overflowed its stack" (0xC00000FD). Ask the linker for the same 8 MiB.
fn main() {
    const STACK: u32 = 8 * 1024 * 1024;
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => println!("cargo:rustc-link-arg-bins=/STACK:{STACK}"),
        Ok("gnu") => println!("cargo:rustc-link-arg-bins=-Wl,--stack,{STACK}"),
        _ => {}
    }
}
