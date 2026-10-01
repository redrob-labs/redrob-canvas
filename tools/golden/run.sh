#!/usr/bin/env bash
# Golden-output harness: regenerate the UPSTREAM's own reference values and diff them against what this
# repository recorded.
#
# Stage 2 of the porting plan. Every constant asserted by the Krita translations in crates/redrob-core came
# from one of the probes in probes/: Krita's own algorithm, extracted with its arithmetic untouched, compiled
# and run. Until this script existed those probes lived in a scratch directory that is reclaimed between
# sessions, so no one could regenerate a single reference value or notice if the pinned upstream changed
# underneath them.
#
# What it does:
#   --check   (default) compile each probe, run it, diff its output against expected/<probe>.txt
#   --update  overwrite expected/<probe>.txt with the current output, for a deliberate pin bump
#   --list    print each probe and whether its dependencies are present
#
# It NEVER passes silently when it has nothing to compare against. A missing library prefix or a missing
# upstream checkout makes the probe that needs it UNRESOLVED and the whole run fail, because a harness that
# reports success when its subject is absent is worse than no harness -- the same rule that keeps the database
# dialect harness out of `npm test`.
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
probes="$here/probes"
expected="$here/expected"
mode="${1:---check}"

# Where the optional upstream prefixes are looked for. These are fetched with `apt-get download` +
# `dpkg-deb -x` into a scratch directory; docs/golden-harness.md carries the recipe.
scratch="${KIROCREW_SCRATCH:-${TMPDIR:-/tmp}}"
boost_prefix="${REDROB_BOOST_PREFIX:-$scratch/boostprefix/usr/include}"
eigen_prefix="${REDROB_EIGEN_PREFIX:-$scratch/eigenprefix/usr/include/eigen3}"
lcms_include="${REDROB_LCMS_PREFIX:-$scratch/lcmsprefix/usr/include}"
lcms_lib="${REDROB_LCMS_LIB:-$scratch/lcmsprefix/usr/lib/x86_64-linux-gnu}"

# probe -> extra compiler flags, and the human name of what must be present.
declare -A needs=(
  [krita-latency-probe]="boost"
  [krita-spline-probe]="eigen"
  [lab-difference-probe]="lcms2"
  [krita-mask-probe]=""
  [krita-antialias-probe]=""
  [krita-gbr-probe]=""
  [krita-spacing-probe]=""
  [krita-abr-probe]=""
  [krita-downscale-probe]=""
)

flags_for() {
  case "$1" in
    boost) [ -d "$boost_prefix/boost" ] && printf -- "-I%s" "$boost_prefix" || return 1 ;;
    eigen) [ -d "$eigen_prefix/Eigen" ] && printf -- "-I%s" "$eigen_prefix" || return 1 ;;
    lcms2) [ -f "$lcms_include/lcms2.h" ] &&
             printf -- "-I%s -L%s -llcms2" "$lcms_include" "$lcms_lib" || return 1 ;;
    "") printf "" ;;
    *) return 1 ;;
  esac
}

if [ "$mode" = "--list" ]; then
  printf "%-26s %-8s %s\n" "probe" "needs" "state"
  for probe in $(printf "%s\n" "${!needs[@]}" | sort); do
    requirement="${needs[$probe]}"
    if flags_for "$requirement" >/dev/null 2>&1; then state="ready"; else state="UNRESOLVED"; fi
    printf "%-26s %-8s %s\n" "$probe" "${requirement:-none}" "$state"
  done
  exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

failures=0
unresolved=0
compared=0

for probe in $(printf "%s\n" "${!needs[@]}" | sort); do
  source="$probes/$probe.cpp"
  if [ ! -f "$source" ]; then
    printf "%-26s MISSING SOURCE\n" "$probe"
    failures=$((failures + 1))
    continue
  fi
  requirement="${needs[$probe]}"
  if ! extra="$(flags_for "$requirement")"; then
    printf "%-26s UNRESOLVED -- %s is not available\n" "$probe" "$requirement"
    unresolved=$((unresolved + 1))
    continue
  fi
  # shellcheck disable=SC2086
  if ! g++ -std=c++17 -O2 -o "$work/$probe" "$source" $extra 2>"$work/$probe.build"; then
    printf "%-26s BUILD FAILED\n" "$probe"
    sed -n '1,6p' "$work/$probe.build"
    failures=$((failures + 1))
    continue
  fi
  if [ -n "$requirement" ] && [ "$requirement" = lcms2 ]; then
    export LD_LIBRARY_PATH="$lcms_lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  fi
  if ! "$work/$probe" >"$work/$probe.out" 2>&1; then
    printf "%-26s RAN AND FAILED\n" "$probe"
    sed -n '1,6p' "$work/$probe.out"
    failures=$((failures + 1))
    continue
  fi

  target="$expected/$probe.txt"
  if [ "$mode" = "--update" ]; then
    cp "$work/$probe.out" "$target"
    printf "%-26s updated\n" "$probe"
    compared=$((compared + 1))
    continue
  fi
  if [ ! -f "$target" ]; then
    printf "%-26s NO RECORDED OUTPUT -- run with --update to record it\n" "$probe"
    failures=$((failures + 1))
    continue
  fi
  if diff -u "$target" "$work/$probe.out" >"$work/$probe.diff"; then
    printf "%-26s matches\n" "$probe"
    compared=$((compared + 1))
  else
    printf "%-26s DIFFERS from the recorded upstream output\n" "$probe"
    sed -n '1,24p' "$work/$probe.diff"
    failures=$((failures + 1))
  fi
done

printf "\n"
if [ "$unresolved" -gt 0 ]; then
  printf "%d probe(s) UNRESOLVED: their upstream dependency is not on this machine.\n" "$unresolved"
  printf "This is a FAILURE, not a pass. See docs/golden-harness.md for the fetch recipe.\n"
fi
if [ "$compared" -eq 0 ]; then
  printf "nothing was compared, so nothing was verified\n"
fi
if [ "$failures" -gt 0 ] || [ "$unresolved" -gt 0 ] || [ "$compared" -eq 0 ]; then
  printf "golden harness: FAILED (%d compared, %d failed, %d unresolved)\n" \
    "$compared" "$failures" "$unresolved"
  exit 1
fi
printf "golden harness: %d probe(s) match the recorded upstream output\n" "$compared"
