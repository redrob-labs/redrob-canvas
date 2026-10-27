#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Build, test and package ONE release tag on this Linux machine.
#
#   build-linux.sh <tag> <checkout> <outdir>
#
# A rehearsal build; the release itself is built by .github/workflows/release.yml on a runner with
# the same steps. Nothing on Linux is code-signed. The binary is linked against THIS machine's glibc,
# so it runs only on distributions at least as new. <tag> may also be origin/<branch>.
# Writes the platform archive, the .deb, the licence files and their SHA-256 sums to <outdir>.
set -euo pipefail
tag="$1"
# Absolute, because the build changes directory into the checkout.
mkdir -p "$3"; out="$(cd "$3" && pwd)"
case "$2" in /*) repo="$2" ;; *) repo="$PWD/$2" ;; esac
export PATH="$HOME/.cargo/bin:$PATH"
target=x86_64-unknown-linux-gnu

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
python3 tools/generate_distribution_artifacts.py --check
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked -p redrob-canvas --target "$target"
"target/$target/release/redrob-canvas" --version

base="redrob-canvas-${version}-linux-x86_64"
stage="$(mktemp -d)"
python3 tools/release/stage.py --target "$target" --version "$version" --out "$stage"
tar -C "$stage" -czf "$out/${base}.tar.gz" .
# The same tree as a package for apt; the system's own uninstall removes it.
python3 tools/release/make_deb.py "$stage" "$version" "$out/${base}.deb"
cp LICENSE-MIT LICENSE-APACHE COPYRIGHT NOTICE THIRD_PARTY_NOTICES.md "$out/"
# redrob-launcher.json: what Redrob Launcher reads to learn this release uses its sign-in. Written
# before the sums so the launcher's quota-free path, which reads the sums file, sees it too.
python3 tools/release_manifest.py --write "$out"
( cd "$out" && sha256sum "${base}.tar.gz" "${base}.deb" \
    LICENSE-MIT LICENSE-APACHE COPYRIGHT NOTICE THIRD_PARTY_NOTICES.md redrob-launcher.json > "${base}-SHA256SUMS.txt" )
python3 tools/release_manifest.py --check "$out"
ls -l "$out"
