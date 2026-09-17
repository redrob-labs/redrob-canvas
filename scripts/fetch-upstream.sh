#!/usr/bin/env bash
#
# Fetches the pinned upstream reference sources into upstream/.
#
# redrob-canvas is NOT a fork of Krita or GIMP. It is first-party Rust and Qt that ports behaviour
# from those two, and `docs/upstream-sources.toml` pins the exact commits it was written against so
# a claim about upstream behaviour can be checked rather than remembered.
#
# Until now that pinning was documentation only: 753 MB of Krita and GIMP source had to be staged by
# hand, `upstream/README.md` said "They intentionally do not download source", and nothing in the
# repository could put a checkout where the build expects one. A new machine could not build, and a
# stale checkout could not be told apart from a correct one.
#
# So this reads the SAME pins the docs declare -- there is no second copy of a commit hash to drift
# -- and produces exactly those trees.
#
# Shallow, single-commit clones: history is not what these are for. Krita's tree alone is ~580 MB
# and its full history is many times that.
#
# Usage:
#   scripts/fetch-upstream.sh            # fetch every pinned source that has a commit
#   scripts/fetch-upstream.sh krita      # just one

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

pins="docs/upstream-sources.toml"
[ -f "$pins" ] || { echo "error: $pins not found" >&2; exit 1; }

# The sources that are actually vendored as trees. gegl is a version floor on a system package and
# redrob_code is a protocol reference, so neither is checked out here -- listing them would create a
# directory nothing reads.
VENDORED=(krita gimp)

read_pin() { # section key -> value
  python3 - "$pins" "$1" "$2" <<'PY'
import re, sys
path, section, key = sys.argv[1], sys.argv[2], sys.argv[3]
text = open(path, encoding="utf-8").read()
# Deliberately a small parser rather than a tomllib dependency: this has to run from a bare
# checkout with nothing installed, and Blender-style vendored Python is not in play here.
match = re.search(rf"^\[{re.escape(section)}\]\s*$(.*?)(?=^\[|\Z)", text, re.M | re.S)
if not match:
    sys.exit(f"section [{section}] not found in {path}")
body = match.group(1)
found = re.search(rf'^{re.escape(key)}\s*=\s*"([^"]+)"\s*$', body, re.M)
if not found:
    sys.exit(f"{section}.{key} not found in {path}")
print(found.group(1))
PY
}

targets=("${VENDORED[@]}")
if [ "$#" -gt 0 ]; then
  targets=("$@")
fi

for name in "${targets[@]}"; do
  found=0
  for known in "${VENDORED[@]}"; do
    [ "$name" = "$known" ] && found=1
  done
  if [ "$found" -eq 0 ]; then
    echo "error: $name is not a vendored source. Known: ${VENDORED[*]}" >&2
    exit 1
  fi

  repo=$(read_pin "$name" repository | tr -d '\r')
  commit=$(read_pin "$name" commit | tr -d '\r')
  target="upstream/$name"

  if [ -d "$target/.git" ]; then
    have=$(git -C "$target" rev-parse HEAD 2>/dev/null || echo "")
    if [ "$have" = "$commit" ]; then
      echo "$name already at $commit"
      continue
    fi
    echo "$name is at ${have:-unknown}, wanted $commit -- refetching"
    rm -rf "$target"
  elif [ -d "$target" ]; then
    # A hand-staged tree with no git metadata -- which is how these arrived before this script
    # existed. It cannot be verified against the pin, so it is replaced rather than trusted.
    echo "$name exists without git metadata, so its commit cannot be verified -- replacing"
    rm -rf "$target"
  fi

  mkdir -p upstream
  echo "fetching $name from $repo at $commit"
  git init --quiet "$target"
  git -C "$target" config core.autocrlf false
  git -C "$target" config core.eol lf
  git -C "$target" remote add origin "$repo"
  git -C "$target" fetch --quiet --depth 1 origin "$commit"
  git -C "$target" checkout --quiet FETCH_HEAD

  actual=$(git -C "$target" rev-parse HEAD)
  if [ "$actual" != "$commit" ]; then
    echo "error: $name expected $commit, checked out $actual" >&2
    exit 1
  fi
  echo "$name at $actual ($(du -sh "$target" 2>/dev/null | cut -f1))"
done
