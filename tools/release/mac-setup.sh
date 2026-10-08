#!/bin/bash
# Redrob Canvas: one-time macOS build toolchain, in the user's home only.
# Rust 1.92.0 (rust-toolchain.toml), CMake + Ninja from PyPI, Qt 6.11.2 clang_64 via the
# same pinned aqtinstall commit the CI workflow uses.
set -euo pipefail
ROOT="$HOME/redrob"
T="$ROOT/.toolchain"
mkdir -p "$T" "$ROOT/logs"
export PATH="$HOME/.cargo/bin:$HOME/Library/Python/3.9/bin:$PATH"
if [ ! -x "$HOME/.cargo/bin/rustup" ]; then
  echo "== rustup"
  curl -sSf https://sh.rustup.rs -o "$T/rustup-init.sh"
  sh "$T/rustup-init.sh" -y --default-toolchain 1.92.0 --profile minimal --no-modify-path
fi
echo "== cmake ninja aqt"
# Released aqtinstall 3.3.0 resolves macOS clang_64 6.11.2; the pinned commit CI uses is the fix
# for the Windows metadata path only, and the Command Line Tools' Python 3.9 cannot build it.
python3 -m pip install --user --quiet 'cmake==3.31.6' 'ninja==1.11.1.4' 'aqtinstall==3.3.0'
if [ ! -x "$T/Qt/6.11.2/macos/bin/qmake" ]; then
  echo "== Qt 6.11.2 clang_64"
  python3 -m aqt install-qt mac desktop 6.11.2 clang_64 --outputdir "$T/Qt"
fi
echo "== done"
