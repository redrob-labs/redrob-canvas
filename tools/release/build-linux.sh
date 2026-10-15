#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Build, test and package ONE release tag on this Linux machine.
#
#   build-linux.sh <tag> <checkout> <outdir>
#
# A rehearsal build; the release itself is built by .github/workflows/release.yml on a runner with
# the same steps. Nothing on Linux is code-signed. The archive is linked against THIS machine's
# glibc, so it runs only on distributions at least as new. <tag> may also be origin/<branch>.
# Writes the platform archive, the corresponding source, the licence files and their SHA-256 sums
# to <outdir>.
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
cmake -S native -B build/qt -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build/qt
ctest --test-dir build/qt --output-on-failure

base="redrob-canvas-${version}-linux-x86_64"
stage="$(mktemp -d)"
cmake --install build/qt --prefix "$stage"
# Qt goes in with the app: until 0.5.5 the archive carried the binary alone and started only where Qt
# 6.11 was already installed system-wide. The script refuses if anything still resolves outside.
python3 tools/release/bundle_linux_qt.py "$stage" "$qt"
tar -C "$stage" -czf "$out/${base}.tar.gz" .
# The same tree as a package for apt; the system's own uninstall removes it.
python3 tools/release/make_deb.py "$stage" "$version" "$out/${base}.deb"
cp build/qt/redrob-canvas-corresponding-source.tar.gz "$out/redrob-canvas-${version}-corresponding-source.tar.gz"
cp THIRD_PARTY_NOTICES.md SOURCE_OFFER.md LICENSE COPYRIGHT "$out/"
# redrob-launcher.json: what Redrob Launcher reads to learn this release uses its sign-in. Written
# before the sums so the launcher's quota-free path, which reads the sums file, sees it too.
python3 tools/release_manifest.py --write "$out"
( cd "$out" && sha256sum "${base}.tar.gz" "${base}.deb" "redrob-canvas-${version}-corresponding-source.tar.gz" \
    THIRD_PARTY_NOTICES.md SOURCE_OFFER.md LICENSE COPYRIGHT redrob-launcher.json > "${base}-SHA256SUMS.txt" )
python3 tools/release_manifest.py --check "$out"
ls -l "$out"
