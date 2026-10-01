#!/usr/bin/env bash
# Diffs every COPIED upstream source against the pinned original, and compares that diff against the one this
# repository recorded.
#
# Stage 2, second harness. Twelve files under crates/redrob-core/src/geometry are Graphite's code taken under
# Apache-2.0, each carrying a header that names its upstream path, the pinned commit, and every change made to
# it -- ending with "Use `git diff --ignore-all-space` to compare against upstream".
#
# That instruction was never executed by anything. scripts/verify-upstream.sh checks the PIN, the attribution,
# the licence and the declared boundary, and its own comments say that for `kind = "code"` the re-sync method is
# a diff against the upstream file -- but nothing performed the diff. So a copied file could drift from the
# original it claims to be, or gain an undeclared change, and every check would still pass.
#
# Unlike the Krita translations there IS a file to diff here, which is exactly why `code` and `translated` are
# separate kinds. This harness is what makes that distinction pay.
#
#   --check   (default) regenerate each diff and compare against recorded/<file>.diff
#   --update  re-record, for a deliberate pin bump or an approved change
#   --list    show each copied file and its declared upstream path
#
# It never passes silently: a missing upstream checkout, a file whose declared path is absent from that
# checkout, or a file with no recorded diff are all FAILURES, not skips.
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
recorded="$here/recorded"
mode="${1:---check}"

geometry="$repo/crates/redrob-core/src/geometry"
checkout="${REDROB_GRAPHITE_CHECKOUT:-$repo/upstream/graphite}"
pin="$(sed -n '/^\[graphite\]/,/^\[/p' "$repo/docs/upstream-sources.toml" |
        sed -n 's/^commit = "\([0-9a-f]\{40\}\)".*/\1/p' | head -1)"

# The declared upstream path, from whichever of the three header phrasings a file uses:
#   "Ported from Graphite `<path>`"        "Ported from Graphite (`<path>`)"
#   "Items extracted from Graphite `<path>`"
declared_path() {
  sed -n '1,12p' "$1" |
    sed -n 's|^//!.*Graphite (\?`\([^`]*\)`.*|\1|p' | head -1
}

collect() {
  for file in "$geometry"/*.rs; do
    path="$(declared_path "$file")"
    [ -n "$path" ] && printf "%s\t%s\n" "$file" "$path"
  done
}

if [ "$mode" = "--list" ]; then
  printf "pin %s\n\n" "${pin:-UNKNOWN}"
  collect | while IFS=$'\t' read -r file path; do
    printf "%-24s %s\n" "$(basename "$file")" "$path"
  done
  exit 0
fi

if [ -z "$pin" ]; then
  printf "could not read graphite's pinned commit from docs/upstream-sources.toml\n"
  exit 1
fi
if [ ! -d "$checkout" ]; then
  printf "no Graphite checkout at %s\n" "$checkout"
  printf "This is a FAILURE, not a pass -- there is nothing to diff against.\n"
  printf "Run: scripts/fetch-upstream.sh graphite\n"
  exit 1
fi
actual_pin="$(git -C "$checkout" rev-parse HEAD 2>/dev/null || echo unknown)"
if [ "$actual_pin" != "$pin" ]; then
  printf "the checkout at %s is at %s, not the pinned %s\n" "$checkout" "$actual_pin" "$pin"
  printf "This is a FAILURE: a diff against the wrong commit proves nothing.\n"
  exit 1
fi

mkdir -p "$recorded"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

failures=0
compared=0

# Comparison is on TOKEN STREAMS, not lines. The file headers name `git diff --ignore-all-space`, and that was
# the first version here -- but it ignores whitespace within a line and not line breaks, and every copied file
# was reformatted from Graphite's hard tabs and long lines to this project's rustfmt. `intersection.rs` reported
# 366 changed lines that were almost entirely re-wrapping, and a real one-line change would have been invisible
# inside that. A check too noisy to detect what it is for is the wrong check.
#
# normalise.py drops comments (ours carry the Apache 4(b) notice we are REQUIRED to add) and emits one token per
# line, so what survives a diff is an identifier, a literal, an operator or a visibility keyword.
normalise() {
  python3 "$here/normalise.py" "$1"
}

while IFS=$'\t' read -r file path; do
  name="$(basename "$file" .rs)"
  original="$checkout/$path"
  if [ ! -f "$original" ]; then
    printf "%-24s DECLARED PATH MISSING from the checkout: %s\n" "$name" "$path"
    failures=$((failures + 1))
    continue
  fi
  if ! normalise "$file" >"$work/ours.tok" || ! normalise "$original" >"$work/theirs.tok"; then
    printf "%-24s COULD NOT NORMALISE\n" "$name"
    failures=$((failures + 1))
    continue
  fi
  diff -u --label "upstream/$path" --label "geometry/$name.rs" \
    "$work/theirs.tok" "$work/ours.tok" >"$work/$name.diff"

  target="$recorded/$name.diff"
  if [ "$mode" = "--update" ]; then
    cp "$work/$name.diff" "$target"
    changed="$(grep -cE '^[+-][^+-]' "$target" || true)"
    total="$(wc -l <"$work/theirs.tok" | tr -d ' ')"
    printf "%-24s recorded (%s of %s tokens differ)\n" "$name" "$changed" "$total"
    compared=$((compared + 1))
    continue
  fi
  if [ ! -f "$target" ]; then
    printf "%-24s NO RECORDED DIFF -- run with --update to record it\n" "$name"
    failures=$((failures + 1))
    continue
  fi
  if diff -q "$target" "$work/$name.diff" >/dev/null; then
    printf "%-24s unchanged against the pinned upstream\n" "$name"
    compared=$((compared + 1))
  else
    printf "%-24s DRIFTED -- its difference from upstream is not the recorded one\n" "$name"
    diff -u "$target" "$work/$name.diff" | sed -n '1,20p'
    failures=$((failures + 1))
  fi
done < <(collect)

printf "\n"
if [ "$compared" -eq 0 ]; then
  printf "no copied file was compared, so nothing was verified\n"
fi
if [ "$failures" -gt 0 ] || [ "$compared" -eq 0 ]; then
  printf "copied-source drift: FAILED (%d compared, %d failed)\n" "$compared" "$failures"
  exit 1
fi
printf "copied-source drift: %d file(s) differ from upstream exactly as recorded\n" "$compared"
