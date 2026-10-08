#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Build, test and package ONE release tag on this Linux machine.
#
#   build-linux.sh <tag> <checkout> <outdir>
#
# Nothing on Linux is code-signed, so this archive is final. It is linked against THIS machine's
# glibc, so it runs only on distributions at least as new; that is an accepted trade for building
# here. <tag> may also be origin/<branch> to rehearse. Writes the platform archive, the
# corresponding source, the licence files and their SHA-256 sums to <outdir>.
set -euo pipefail
tag="$1"
# Absolute, because the build changes directory into the checkout.
mkdir -p "$3"; out="$(cd "$3" && pwd)"
case "$2" in /*) repo="$2" ;; *) repo="$PWD/$2" ;; esac
qt="${REDROB_QT_PREFIX:-$HOME/Qt/6.11.2/gcc_64}"
export PATH="$HOME/.cargo/bin:$qt/bin:$PATH"
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
ctest --test-dir build/qt --output-on-failure

base="redrob-canvas-${version}-linux-x86_64"
stage="$(mktemp -d)"
cmake --install build/qt --prefix "$stage"
tar -C "$stage" -czf "$out/${base}.tar.gz" .
cp build/qt/redrob-canvas-corresponding-source.tar.gz "$out/redrob-canvas-${version}-corresponding-source.tar.gz"
cp THIRD_PARTY_NOTICES.md SOURCE_OFFER.md LICENSE COPYRIGHT "$out/"
( cd "$out" && sha256sum "${base}.tar.gz" "redrob-canvas-${version}-corresponding-source.tar.gz" \
    THIRD_PARTY_NOTICES.md SOURCE_OFFER.md LICENSE COPYRIGHT > "${base}-SHA256SUMS.txt" )
ls -l "$out"
