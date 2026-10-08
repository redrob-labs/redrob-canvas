#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Build, test and package ONE release tag on this macOS machine, unsigned.
#
#   build-macos.sh <tag> <checkout> <outdir>
#
# <tag> may also be a branch as origin/<name>, to rehearse a build before tagging; the archive is
# then named for the Cargo.toml version.
# The release workflow signs, notarizes and staples what this produces; nothing here touches a
# certificate. tools/release/local-release.sh runs this over SSH. The checkout is reused between
# releases so Cargo and CMake build incrementally; it is moved to the tag, never edited.
set -euo pipefail
tag="$1"; repo="$2"; out="$3"
qt="${REDROB_QT_PREFIX:-$HOME/redrob/.toolchain/Qt/6.11.2/macos}"
export PATH="$HOME/.cargo/bin:$HOME/Library/Python/3.9/bin:$qt/bin:$PATH"
export CMAKE_PREFIX_PATH="$qt"

if [ ! -d "$repo/.git" ]; then
  git clone --quiet https://github.com/redrob-labs/redrob-canvas.git "$repo"
fi
cd "$repo"
git fetch --quiet --force --tags origin '+refs/heads/*:refs/remotes/origin/*'
git checkout --quiet --force --detach "$tag"
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)"
case "$tag" in
  v*) [ "${tag#v}" = "$version" ] || { echo "$tag but Cargo.toml says $version" >&2; exit 1; } ;;
esac
echo "== $(git rev-parse --short HEAD) $tag"

cargo fetch --locked
cmake -S native -B build/qt -G Ninja -DCMAKE_BUILD_TYPE=Release \
  -DREDROB_ENABLE_GEGL=OFF -DREDROB_ENABLE_KRITA=OFF
cmake --build build/qt
# The smoke tests run offscreen, so they need no window server and pass over SSH.
ctest --test-dir build/qt --output-on-failure

work="$(mktemp -d)"
base="redrob-canvas-${version}-macos-arm64"
stage="$work/$base"
cmake --install build/qt --prefix "$stage"
app="$stage/Redrob Canvas.app"
test -d "$app"
macdeployqt "$app" -qmldir="$PWD/qml"

plist="$app/Contents/Info.plist"
for key in CFBundleShortVersionString CFBundleVersion; do
  got="$(plutil -extract "$key" raw -o - "$plist")"
  if [ "$got" != "$version" ]; then
    echo "Info.plist $key is $got, Cargo.toml is $version" >&2
    exit 1
  fi
done
# The archive is the whole staging tree, so the licence, notices and corresponding source travel
# with the binary as the GPL requires. ditto keeps the bundle's structure intact for signing.
mkdir -p "$out"
rm -f "$out/${base}.unsigned.zip"
( cd "$work" && ditto -c -k --sequesterRsrc --keepParent "$base" "$out/${base}.unsigned.zip" )
rm -rf "$work"
ls -l "$out/${base}.unsigned.zip"
