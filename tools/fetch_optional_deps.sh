#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Fetch the OPTIONAL native dependencies (babl, lcms2, GEGL) into a private prefix,
# without root.
#
# Why this exists. The adapters behind REDROB_ENABLE_BABL, REDROB_ENABLE_LCMS and
# REDROB_ENABLE_GEGL are OFF by default, so an ordinary build and CI never need any
# of these. But an adapter that has never been compiled against its library is not
# a verified adapter, and the sandbox this is developed in has no root: `sudo
# apt-get install` fails with "the no new privileges flag is set". Extracting the
# same Debian packages into a prefix needs no privilege and gives a real
# pkg-config module, headers and shared library.
#
# It is committed rather than kept as one person's shell history because a
# verification nobody else can reproduce is a claim, not evidence.
#
#   tools/fetch_optional_deps.sh            # everything not already present
#   tools/fetch_optional_deps.sh babl       # just one
#   eval "$(tools/fetch_optional_deps.sh --env)"   # print the exports and stop
#
# On a machine WITH root, prefer the distribution packages and skip this:
#   sudo apt-get install libbabl-dev liblcms2-dev libgegl-dev

set -euo pipefail

PREFIX="${REDROB_OPTIONAL_DEPS_PREFIX:-${KIROCREW_SCRATCH:-${TMPDIR:-/tmp}}/redrob-native-deps}"
ARCH="$(dpkg --print-architecture)"
MULTIARCH="$(dpkg-architecture -qDEB_HOST_MULTIARCH 2>/dev/null || echo "x86_64-linux-gnu")"
PC_DIR="$PREFIX/usr/lib/$MULTIARCH/pkgconfig"
LIB_DIR="$PREFIX/usr/lib/$MULTIARCH"

# Package sets per pkg-config module. The runtime library is listed beside the -dev
# package because dpkg-deb extracts exactly what it is given -- there is no
# dependency resolution here, which is the trade for needing no privilege.
declare -A PACKAGES=(
  [babl]="libbabl-dev libbabl-0.1-0"
  [lcms2]="liblcms2-dev liblcms2-2"
  [gegl]="libgegl-dev libgegl-0.4-0t64 libgegl-common libjson-glib-dev libjson-glib-1.0-0 libjson-glib-1.0-common"
  # libmypaint is the fourth bridge, found in 1a.7: Krita does not reimplement MyPaint,
  # its plugin includes <libmypaint/mypaint-brush.h> and links the library.
  [mypaint]="libmypaint-dev libmypaint-1.5-1 libmypaint-common libjson-c-dev"
)
declare -A MODULES=(
  [babl]="babl-0.1"
  [lcms2]="lcms2"
  [gegl]="gegl-0.4"
  [mypaint]="libmypaint"
)

print_env () {
  echo "export PKG_CONFIG_PATH=\"$PC_DIR\${PKG_CONFIG_PATH:+:\$PKG_CONFIG_PATH}\""
  echo "export LD_LIBRARY_PATH=\"$LIB_DIR\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}\""
  # babl and GEGL both look for their operation plug-ins at a path compiled into
  # the library. Without these they still work, but fall back to reference
  # conversions and warn loudly about a broken installation.
  echo "export BABL_PATH=\"$LIB_DIR/babl-0.1\""
  echo "export GEGL_PATH=\"$LIB_DIR/gegl-0.4\""
}

if [ "${1:-}" = "--env" ]; then
  print_env
  exit 0
fi

targets=("$@")
if [ ${#targets[@]} -eq 0 ]; then
  targets=(babl lcms2 gegl mypaint)
fi

mkdir -p "$PREFIX" "$PC_DIR"
download_dir="$PREFIX/.debs"
mkdir -p "$download_dir"

for target in "${targets[@]}"; do
  module="${MODULES[$target]:-}"
  if [ -z "$module" ]; then
    echo "error: unknown target '$target'; known: ${!MODULES[*]}" >&2
    exit 2
  fi

  # Already satisfied system-wide? Then do nothing: a prefix copy shadowing a
  # working system library is a way to test something the user will never ship.
  if PKG_CONFIG_PATH="" pkg-config --exists "$module" 2>/dev/null; then
    echo "$target: already present system-wide ($(PKG_CONFIG_PATH="" pkg-config --modversion "$module"))"
    continue
  fi
  if PKG_CONFIG_PATH="$PC_DIR" pkg-config --exists "$module" 2>/dev/null; then
    echo "$target: already in the prefix ($(PKG_CONFIG_PATH="$PC_DIR" pkg-config --modversion "$module"))"
    continue
  fi

  echo "$target: downloading ${PACKAGES[$target]}"
  ( cd "$download_dir" && apt-get download ${PACKAGES[$target]} >/dev/null 2>&1 ) || {
    echo "error: apt-get download failed for $target" >&2
    echo "       Check that the package names still exist for $ARCH:" >&2
    echo "         apt-cache policy ${PACKAGES[$target]}" >&2
    exit 1
  }
  for deb in "$download_dir"/*.deb; do
    dpkg-deb -x "$deb" "$PREFIX"
  done
  rm -f "$download_dir"/*.deb
done

# Every .pc file in the prefix points at /usr, which is not where these live now.
# Rewriting the three path variables is what makes pkg-config resolve against the
# extracted tree. Done for all of them each run, so a file added by a later target
# is corrected too.
shopt -s nullglob
for pc in "$PC_DIR"/*.pc; do
  sed -i \
    -e "s|^prefix=.*|prefix=$PREFIX/usr|" \
    -e "s|^libdir=.*|libdir=$LIB_DIR|" \
    -e "s|^includedir=.*|includedir=$PREFIX/usr/include|" \
    "$pc"
done
shopt -u nullglob

echo
echo "prefix: $PREFIX"
for target in babl lcms2 gegl mypaint; do
  module="${MODULES[$target]}"
  if PKG_CONFIG_PATH="" pkg-config --exists "$module" 2>/dev/null; then
    printf '  %-6s system   %s\n' "$target" "$(PKG_CONFIG_PATH="" pkg-config --modversion "$module")"
  elif PKG_CONFIG_PATH="$PC_DIR" pkg-config --exists "$module" 2>/dev/null; then
    printf '  %-6s prefix   %s\n' "$target" "$(PKG_CONFIG_PATH="$PC_DIR" pkg-config --modversion "$module")"
  else
    printf '  %-6s MISSING\n' "$target"
  fi
done
echo
echo "To build against these:"
echo
print_env | sed 's/^/  /'
