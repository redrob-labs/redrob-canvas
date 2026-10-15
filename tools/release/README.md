# Release packaging

The release is `.github/workflows/release.yml`: every platform is built, signed and published on GitHub
runners (see the README's *Releases* section). The pieces here are what that workflow calls, plus one
rehearsal script.

- `stage.py` turns the built `redrob-canvas` binary into the per-platform tree every archive is made
  from: `bin/` plus the licence set on Linux and Windows, `Redrob Canvas.app` (with its `Info.plist`,
  icon and the licences inside) on macOS. Shared by all three legs.
- `make_deb.py` packages the Linux tree as a `.deb` under `/opt/redrob-canvas`, with the menu entry,
  icons and a `Depends` line worked out from the binary.
- `build-linux.sh <tag> <checkout> <outdir>` rehearses the Linux leg on our own machine, unsigned:
  gates, `cargo build --release`, stage, `.tar.gz`, `.deb`, licences, launcher manifest, sums.

There is no Qt, CMake or aqtinstall anywhere here any more: the shell is egui and a leg is one
`cargo build --release -p redrob-canvas --target <triple>`. The macOS disk image and the Windows setup
live in `installer/`.

The release is exactly four targets (`tools/release_matrix.py`):

| Target        | Assets                                                              | Built by                    |
|---------------|---------------------------------------------------------------------|-----------------------------|
| macOS arm64   | `redrob-canvas-<v>-macos-arm64.zip`, `redrob-canvas-<v>-macos-arm64.dmg` | `build` leg (`macos-15`), signed and notarized |
| Linux x86_64  | `redrob-canvas-<v>-linux-x86_64.tar.gz`, `redrob-canvas-<v>-linux-x86_64.deb` | `build` leg (`ubuntu-24.04`) |
| Windows x64   | `redrob-canvas-<v>-windows-x64-portable.zip`, `redrob-canvas-<v>-windows-x64-setup.exe` | `build` leg (`windows-2025`), Azure Artifact Signing |

No other package format or architecture is built. The web zip is the only asset Redrob Launcher
does not install; a host unzips it behind any static web server.

## Linux

Rust 1.95 (`rust-toolchain.toml`), `dpkg-dev`, and the development headers egui, wgpu and rfd link
against: `libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev libgtk-3-dev`.
