#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Build the Redrob Canvas disk image: the app, an Applications link, and the uninstaller, on a branded
# background. Runs on macOS.
#
#   make-dmg.sh --app "<path>/Redrob Canvas.app" --out <file.dmg> --version X.Y.Z
#               [--licenses <dir>] [--sign-identity <id>]
#
# --licenses is the build's share/redrob-canvas/licenses folder (licence, notices, source offer); it is
# copied in as "Licenses", as the zip carries it beside the app.
#
# Needs dmgbuild (pinned: python3 -m pip install --user dmgbuild==1.6.5), which writes the Finder
# layout itself, so no GUI session or automation permission is needed (dmg_settings.py).
#
# With --sign-identity the uninstaller and the image are signed (hardened runtime, timestamp). The app
# must already be signed, notarized and stapled: the image is built around it, not into it. The release
# workflow then notarizes and staples the image itself.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
app="" out="" version="" identity="" licenses=""
while [ $# -gt 0 ]; do
  case "$1" in
    --app) app="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --version) version="$2"; shift 2 ;;
    --licenses) licenses="$2"; shift 2 ;;
    --sign-identity) identity="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ -d "$app" ] || { echo "--app must be the Redrob Canvas.app bundle" >&2; exit 2; }
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "--version must be X.Y.Z" >&2; exit 2; }
[ -n "$out" ] || { echo "--out is required" >&2; exit 2; }
icns="$app/Contents/Resources/AppIcon.icns"
[ -f "$icns" ] || { echo "the app carries no Contents/Resources/AppIcon.icns" >&2; exit 2; }
python3 -c 'import dmgbuild' 2>/dev/null || { echo "dmgbuild is missing: python3 -m pip install --user dmgbuild==1.6.5" >&2; exit 2; }

work="$(mktemp -d "${TMPDIR:-/tmp}/redrob-dmg.XXXXXX")"
trap 'rm -rf "$work"' EXIT

# The uninstaller: a script in a bundle, so it opens from Finder with the product's icon.
un="$work/Uninstall Redrob Canvas.app"
mkdir -p "$un/Contents/MacOS" "$un/Contents/Resources"
cp "$here/uninstaller/uninstall.sh" "$un/Contents/MacOS/Uninstall Redrob Canvas"
chmod 755 "$un/Contents/MacOS/Uninstall Redrob Canvas"
cp "$icns" "$un/Contents/Resources/AppIcon.icns"
cat > "$un/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>Uninstall Redrob Canvas</string>
  <key>CFBundleIdentifier</key><string>ai.redrob.canvas.uninstaller</string>
  <key>CFBundleName</key><string>Uninstall Redrob Canvas</string>
  <key>CFBundleDisplayName</key><string>Uninstall Redrob Canvas</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHumanReadableCopyright</key><string>Copyright (C) 2026 Janghoon Lee (Redrob) and contributors</string>
</dict>
</plist>
PLIST
if [ -n "$identity" ]; then
  codesign --force --options runtime --timestamp --sign "$identity" "$un"
  codesign --verify --strict --verbose=2 "$un"
fi

mkdir -p "$(dirname "$out")"
rm -f "$out"
extra=()
if [ -n "$licenses" ]; then
  [ -f "$licenses/LICENSE" ] || { echo "--licenses has no LICENSE: $licenses" >&2; exit 2; }
  extra=(-D licenses="$licenses")
fi
python3 -m dmgbuild -s "$here/dmg_settings.py" \
  -D app="$app" -D uninstaller="$un" -D here="$here" ${extra[@]+"${extra[@]}"} \
  "Redrob Canvas" "$out"
if [ -n "$identity" ]; then
  codesign --force --timestamp --sign "$identity" "$out"
  codesign --verify --verbose=2 "$out"
fi
hdiutil verify -quiet "$out"
echo "built $out ($(du -h "$out" | cut -f1))"
