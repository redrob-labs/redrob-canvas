#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Release a pushed tag: build macOS and Windows on our own machines, then let CI sign them.
#
#   tools/release/local-release.sh v0.5.3
#
# 1. Checks the tag the way the workflow does: on main, matching Cargo.toml, with a CHANGELOG entry.
# 2. Builds and tests Linux here, and macOS and Windows on the Mac and the PC over SSH, all three
#    AT THE SAME TIME. macOS and Windows come back unsigned; Linux is never signed.
# 3. Creates the DRAFT release and uploads the Linux archive, the corresponding source, the
#    licence files and the two unsigned archives.
# 4. Starts .github/workflows/release.yml for the tag. That workflow signs and notarizes macOS,
#    signs Windows, and checks the draft carries everything. Nobody publishes but a person: the
#    draft stays a draft.
#
# The Linux archive is linked against this machine's glibc and runs only on distributions at
# least as new. That was accepted in exchange for building here.
#
# The machines come from the environment, or from ~/.config/redrob-canvas/release.env, never
# from this file -- addresses and user names are not the repository's business:
#   REDROB_MAC_HOST=user@host   REDROB_MAC_ROOT=redrob            (relative to the remote home)
#   REDROB_WIN_HOST=user@host   REDROB_WIN_ROOT=C:/Users/me/redrob   (forward slashes: scp and PowerShell both take them)
set -euo pipefail

tag="${1:?usage: local-release.sh vMAJOR.MINOR.PATCH}"
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "not a release tag: $tag" >&2; exit 2; }
version="${tag#v}"
config="${XDG_CONFIG_HOME:-$HOME/.config}/redrob-canvas/release.env"
# shellcheck disable=SC1090
[ -f "$config" ] && . "$config"
: "${REDROB_MAC_HOST:?set REDROB_MAC_HOST (see the header of this script)}"
: "${REDROB_WIN_HOST:?set REDROB_WIN_HOST (see the header of this script)}"
mac_root="${REDROB_MAC_ROOT:-redrob}"
win_root="${REDROB_WIN_ROOT:?set REDROB_WIN_ROOT (see the header of this script)}"
repo="redrob-labs/redrob-canvas"
here="$(cd "$(dirname "$0")" && pwd)"
ssh_opts=(-F /dev/null -o BatchMode=yes -o ConnectTimeout=15 -o ServerAliveInterval=30)
out="${TMPDIR:-/tmp}/redrob-canvas-release-$version"
mkdir -p "$out"

echo "== checking $tag"
git fetch --quiet --force origin main "refs/tags/$tag:refs/tags/$tag"
commit="$(git rev-parse "$tag^{commit}")"
git merge-base --is-ancestor "$commit" origin/main \
  || { echo "$tag ($commit) is not on origin/main" >&2; exit 1; }
crate="$(git show "$commit:Cargo.toml" | sed -n 's/^version = "\(.*\)"$/\1/p' | head -n1)"
[ "$crate" = "$version" ] || { echo "Cargo.toml at $tag says $crate" >&2; exit 1; }
git show "$commit:CHANGELOG.md" | grep -q "^## $version " \
  || { echo "CHANGELOG.md at $tag has no '## $version' section" >&2; exit 1; }
# The workflow leaves Windows out until signing is configured; this script builds it, so it
# refuses rather than upload an archive nobody will sign.
WINDOWS_SIGNING_READY="$(gh variable get WINDOWS_SIGNING_READY -R "$repo" 2>/dev/null || true)"
export WINDOWS_SIGNING_READY
[ "$WINDOWS_SIGNING_READY" = "true" ] \
  || { echo "WINDOWS_SIGNING_READY is not true on $repo; Windows cannot be signed" >&2; exit 1; }
if gh release view "$tag" -R "$repo" --json isDraft -q .isDraft 2>/dev/null | grep -qx false; then
  echo "$tag is already published" >&2; exit 1
fi

echo "== building Linux, macOS and Windows in parallel"
linux_out="$out/linux"
rm -rf "$linux_out"
"$here/build-linux.sh" "$tag" "${REDROB_LINUX_CHECKOUT:-$HOME/redrob/redrob-canvas}" "$linux_out" \
  > "$out/linux.log" 2>&1 &
linux_job=$!
scp "${ssh_opts[@]}" -q "$here/build-macos.sh" "$REDROB_MAC_HOST:$mac_root/build-macos.sh"
scp "${ssh_opts[@]}" -q "$here/build-windows.ps1" "$REDROB_WIN_HOST:$win_root/build-windows.ps1"
ssh "${ssh_opts[@]}" "$REDROB_MAC_HOST" \
  "bash $mac_root/build-macos.sh $tag $mac_root/redrob-canvas $mac_root/out" \
  > "$out/macos.log" 2>&1 &
mac_job=$!
ssh "${ssh_opts[@]}" "$REDROB_WIN_HOST" \
  "powershell -NoProfile -ExecutionPolicy Bypass -File $win_root/build-windows.ps1 -Tag $tag -Checkout $win_root/redrob-canvas -Out $win_root/out" \
  > "$out/windows.log" 2>&1 &
win_job=$!
failed=0
wait "$linux_job" || { echo "Linux build failed; last lines of $out/linux.log:" >&2; tail -n 30 "$out/linux.log" >&2; failed=1; }
wait "$mac_job" || { echo "macOS build failed; last lines of $out/macos.log:" >&2; tail -n 30 "$out/macos.log" >&2; failed=1; }
wait "$win_job" || { echo "Windows build failed; last lines of $out/windows.log:" >&2; tail -n 30 "$out/windows.log" >&2; failed=1; }
[ "$failed" -eq 0 ] || exit 1

mac_zip="redrob-canvas-$version-macos-arm64.unsigned.zip"
win_zip="redrob-canvas-$version-windows-x86_64.unsigned.zip"
scp "${ssh_opts[@]}" -q "$REDROB_MAC_HOST:$mac_root/out/$mac_zip" "$out/"
scp "${ssh_opts[@]}" -q "$REDROB_WIN_HOST:$win_root/out/$win_zip" "$out/"
ls -l "$out/$mac_zip" "$out/$win_zip"

echo "== draft release"
if ! gh release view "$tag" -R "$repo" >/dev/null 2>&1; then
  notes="$out/notes.md"
  cat > "$notes" <<NOTES
Release notes for this version are in [CHANGELOG.md](https://github.com/$repo/blob/$tag/CHANGELOG.md).

Builds: $(python3 "$here/../release_matrix.py" --notes).

This release is GPL-3.0-or-later. The corresponding source for these binaries is attached as
\`*-corresponding-source.tar.gz\`, together with \`SOURCE_OFFER.md\` and the complete
\`THIRD_PARTY_NOTICES.md\`. Verify downloads against the attached SHA256SUMS files.
NOTES
  gh release create "$tag" -R "$repo" --draft --verify-tag \
    --title "Redrob Canvas $tag" --notes-file "$notes"
fi
gh release upload "$tag" -R "$repo" --clobber "$linux_out"/* "$out/$mac_zip" "$out/$win_zip"

echo "== CI: signing and checks"
gh workflow run release.yml -R "$repo" --ref main -f tag="$tag"
sleep 5
gh run list -R "$repo" --workflow release.yml --limit 1 --json url -q '.[0].url'
