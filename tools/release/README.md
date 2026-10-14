# Rehearsal builds

The release is `.github/workflows/release.yml`: every platform is built, signed and published on GitHub
runners (see the README's *Releases* section). The scripts here build the same thing on our own
machines, unsigned, to rehearse a build before tagging: `build-linux.sh`, `build-macos.sh` and
`build-windows.ps1` each take a tag or `origin/<branch>`, a checkout directory and an output directory.
`bundle_linux_qt.py` and `make_deb.py` are shared with the workflow's Linux leg.

## Linux

Qt 6.11.2 `gcc_64` in `~/Qt/6.11.2/gcc_64` (or set `REDROB_QT_PREFIX`), Rust 1.92.0, CMake and
Ninja.

## Mac (Apple silicon, Command Line Tools is enough)

Run `mac-setup.sh` once on the Mac. It installs Rust 1.92.0, CMake, Ninja and Qt 6.11.2 under the
user's home, nothing system-wide. Xcode is not needed: signing and notarization happen in CI.

## Windows PC

Once: Visual Studio with the C++ workload, Git, Python 3, Ninja on `PATH`, Rust 1.92.0 through
rustup, and Qt 6.11.2 `win64_msvc2022_64` in `<REDROB_WIN_ROOT>\.toolchain\Qt` from aqtinstall at
the commit `release.yml` pins (released aqtinstall cannot resolve Windows Qt 6.11).
