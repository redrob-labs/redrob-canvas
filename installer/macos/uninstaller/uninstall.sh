#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Uninstall Redrob Canvas.app -- the macOS uninstaller shipped on the disk image.
#
# macOS has no uninstall entry to register, so this is what removes Canvas: it asks first, moves the app
# to the Trash (so a mistake is one drag to undo), and only when asked, deletes what Canvas keeps in the
# Library. It never closes a running Canvas, which may hold unsaved work.
#
#   (no arguments)              the dialogs
#   --silent [--delete-data]    no dialogs, for scripted removal and for tests; exit code is the result
#
# The app keeps its state in ~/Library/Application Support/Redrob Canvas (apps/redrob-canvas
# services::config_dir). The "Redrob/Redrob Canvas" paths are where the 0.6 Qt shell kept its settings,
# console key and AI engine; they are removed too so an upgrade-then-uninstall leaves nothing behind.
# The shared "Redrob" folders hold other Redrob apps' data and are left alone.
set -u

silent=0
delete_data=0
for arg in "$@"; do
  case "$arg" in
    --silent) silent=1 ;;
    --delete-data) delete_data=1 ;;
  esac
done

TITLE="Uninstall Redrob Canvas"
ICON="$(cd "$(dirname "$0")/../Resources" && pwd)/AppIcon.icns"

# One AppleScript dialog. Prints the button pressed; empty when the person pressed Cancel or Esc.
ask() {  # message, default button, buttons...
  local message="$1" default="$2"; shift 2
  local buttons="" b
  for b in "$@"; do buttons="${buttons:+$buttons, }\"$b\""; done
  local icon_clause="with icon caution"
  [ -f "$ICON" ] && icon_clause="with icon (POSIX file \"$ICON\" as alias)"
  osascript - "$message" <<OSA 2>/dev/null
on run argv
  try
    set r to display dialog (item 1 of argv) with title "$TITLE" buttons {$buttons} default button "$default" $icon_clause
    return button returned of r
  on error number -128
    return ""
  end try
end run
OSA
}

say_done() {  # message
  [ "$silent" -eq 1 ] && return 0
  ask "$1" "OK" "OK" >/dev/null
}

apps=()
for candidate in "/Applications/Redrob Canvas.app" "$HOME/Applications/Redrob Canvas.app"; do
  [ -d "$candidate" ] && apps+=("$candidate")
done

if [ "${#apps[@]}" -eq 0 ]; then
  say_done "Redrob Canvas is not in Applications, so there is nothing to remove."
  exit 3
fi

# Only a Canvas started from a copy being removed blocks it; one opened from a download folder does not.
for app in "${apps[@]}"; do
  if pgrep -f "^$app/Contents/MacOS/" >/dev/null 2>&1; then
    say_done "Redrob Canvas is open. Save your work, quit it, then open the uninstaller again."
    exit 4
  fi
done

if [ "$silent" -eq 0 ]; then
  answer="$(ask "Remove Redrob Canvas from this Mac? It goes to the Trash. Your images are not touched." \
    "Uninstall" "Cancel" "Uninstall")"
  [ "$answer" = "Uninstall" ] || exit 1
  answer="$(ask "Also delete its settings, the console connection and the AI engine? Keep them if you may install Redrob Canvas again." \
    "Keep" "Delete" "Keep")"
  [ -n "$answer" ] || exit 1
  [ "$answer" = "Delete" ] && delete_data=1
fi

# To the Trash, not deleted: a name already in the Trash gets a timestamp rather than being replaced.
trash="$HOME/.Trash"
mkdir -p "$trash"
for app in "${apps[@]}"; do
  target="$trash/Redrob Canvas.app"
  [ -e "$target" ] && target="$trash/Redrob Canvas $(date +%Y-%m-%d\ %H.%M.%S).app"
  if ! mv "$app" "$target" 2>/dev/null; then
    say_done "Redrob Canvas could not be moved to the Trash. Drag it from Applications to the Trash instead."
    exit 5
  fi
done

if [ "$delete_data" -eq 1 ]; then
  defaults delete "com.redrob.Redrob Canvas" >/dev/null 2>&1  # also flushes cfprefsd's cached copy
  for path in \
    "$HOME/Library/Application Support/Redrob Canvas" \
    "$HOME/Library/Preferences/com.redrob.Redrob Canvas.plist" \
    "$HOME/Library/Application Support/Redrob/Redrob Canvas" \
    "$HOME/Library/Preferences/Redrob/Redrob Canvas" \
    "$HOME/Library/Caches/Redrob/Redrob Canvas" \
    "$HOME/Library/Saved Application State/ai.redrob.canvas.savedState"; do
    rm -rf -- "$path"
  done
  say_done "Redrob Canvas is in the Trash, and its settings and AI engine were deleted."
else
  say_done "Redrob Canvas is in the Trash. Its settings stay on this Mac for a later install."
fi
exit 0
