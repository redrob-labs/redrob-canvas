# Release machines

`local-release.sh` builds macOS and Windows on our own machines over SSH, then CI signs them. See
the README's *Releases* section for the whole flow.

## This machine

`gh` signed in with push access to the repository, SSH keys accepted by both build machines, and:

```sh
# ~/.config/redrob-canvas/release.env -- not in the repository
REDROB_MAC_HOST=user@mac-host
REDROB_MAC_ROOT=redrob                  # relative to the Mac user's home
REDROB_WIN_HOST=user@windows-host
REDROB_WIN_ROOT=C:/Users/user/redrob    # forward slashes
```

## Mac (Apple silicon, Command Line Tools is enough)

Run `mac-setup.sh` once on the Mac. It installs Rust 1.92.0, CMake, Ninja and Qt 6.11.2 under the
user's home, nothing system-wide. Xcode is not needed: signing and notarization happen in CI.

## Windows PC

Once: Visual Studio with the C++ workload, Git, Python 3, Ninja on `PATH`, Rust 1.92.0 through
rustup, and Qt 6.11.2 `win64_msvc2022_64` in `<REDROB_WIN_ROOT>\.toolchain\Qt` from aqtinstall at
the commit `release-ci.yml` pins (released aqtinstall cannot resolve Windows Qt 6.11).

## Rehearsal

Both build scripts take a branch as `origin/<name>` instead of a tag, to check a build on the
machines before tagging. `local-release.sh` itself only takes a tag.
