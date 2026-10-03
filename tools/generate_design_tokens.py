#!/usr/bin/env python3
"""Generate qml/RedrobTokens.qml from the vendored design tokens.

WHY A VENDORED COPY. Every other Redrob product installs `@redrob-labs/ui` and reads its values through
`var(--surface-base)`. This one cannot: there is no Node in this repository's build, and QML has no
`var()` to resolve. The delivery anticipates that -- it ships `dist/styles/tokens.json` with every
`var()` chain already resolved per theme, precisely so a consumer that cannot evaluate CSS still gets
the same numbers. That file is vendored under `third_party/redrob-ui/` and pinned by sha256 in
DESIGN_SYSTEM_PIN.json, so a drifted copy fails a check instead of silently repainting the app.

WHAT IT EMITS. A plain `QtObject` rather than a `pragma Singleton`: a singleton needs a `qmldir` and an
import path, and `native/qt/CMakeLists.txt` registers QML through `qt_add_resources`, not
`qt_add_qml_module`. One instance in `Main.qml` reaches every binding without touching the build.

Both themes are emitted and chosen by the `dark` property at runtime, because the shipped values differ
per theme and a build flag cannot know which one the window is in.

Run with --check to verify the committed file is what the tokens produce; that is what CI calls.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
TOKENS = ROOT / "third_party/redrob-ui/tokens.json"
OUT = ROOT / "qml/RedrobTokens.qml"
PIN = ROOT / "DESIGN_SYSTEM_PIN.json"

# The families this app paints with. Product RAMPS are left out -- 70 properties for shades no editor
# chrome uses -- but each product's MARK is kept, because the app icon's band colour is one of them.
KEEP_PREFIX = (
    "surface-", "border-", "ink-", "action-", "focus-ring", "status-", "overlay-",
    "gray-", "blue-", "redrob-", "accent-",
)
KEEP_EXACT = {f"product-{name}" for name in
              ("router", "chat", "code", "desk", "office", "browser", "design")}


def camel(name: str) -> str:
    head, *rest = name.split("-")
    return head + "".join(part[:1].upper() + part[1:] for part in rest)


def wanted(token: dict) -> bool:
    if token["kind"] != "color":
        return False
    name = token["name"]
    if name in KEEP_EXACT:
        return True
    if any(name.startswith(p) for p in KEEP_PREFIX):
        # product-code-3 and friends start with no kept prefix, so only the ramps are excluded here.
        return True
    return False


def render() -> str:
    data = json.loads(TOKENS.read_text(encoding="utf-8"))
    digest = hashlib.sha256(TOKENS.read_bytes()).hexdigest()
    rows = [t for t in data["tokens"] if wanted(t)]
    rows.sort(key=lambda t: t["name"])

    lines = [
        "// GENERATED FROM THE REDROB DESIGN SYSTEM -- DO NOT EDIT.",
        f"// source: third_party/redrob-ui/tokens.json  sha256 {digest[:16]}…",
        "//",
        "// Regenerate with: python3 tools/generate_design_tokens.py",
        "// Verify with:     python3 tools/generate_design_tokens.py --check",
        "//",
        "// A value changed here is a value that disagrees with every other Redrob product. The numbers",
        "// are the delivery's own, resolved per theme by its generator, not re-derived here.",
        "",
        "import QtQuick",
        "",
        "QtObject {",
        "    // Which ground the window is on. Canvas paints dark, and every binding below follows this",
        "    // rather than a build flag: the app can be dark while the desktop is light.",
        "    property bool dark: true",
        "",
    ]
    for token in rows:
        light, dark = token["light"], token["dark"]
        prop = camel(token["name"])
        if light == dark:
            lines.append(f'    readonly property color {prop}: "{light}"')
        else:
            lines.append(f'    readonly property color {prop}: dark ? "{dark}" : "{light}"')
    lines += ["}", ""]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true",
                        help="fail if the committed file differs from what the tokens produce")
    args = parser.parse_args()

    pin = json.loads(PIN.read_text(encoding="utf-8"))
    # Tool icons are vendored and pinned the same way as tokens, so they share this gate.
    icons = pin.get("vendored_icons", {}).get("files", [])
    for asset in pin["vendored_tokens"] + icons:
        path = ROOT / asset["file"]
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != asset["sha256"]:
            print(f"{asset['file']} does not match its pin.\n"
                  f"  expected {asset['sha256']}\n  actual   {actual}\n"
                  f"  Re-copy it from @redrob-labs/ui {pin['version']} at {asset['delivery_path']},"
                  f" or update the pin deliberately.", file=sys.stderr)
            return 1

    # Our own glyphs carry no hash — there is no upstream to drift from — but a missing or
    # off-geometry one is worse than a drifted hash: Qt renders an absent SVG as an EMPTY button,
    # which builds and runs without a word. So they get an existence and geometry gate instead.
    for rel in pin.get("local_icons", {}).get("files", []):
        path = ROOT / rel
        if not path.is_file():
            print(f"{rel} is listed in local_icons but is not on disk. A missing glyph renders as an"
                  " empty button with no error.", file=sys.stderr)
            return 1
        head = path.read_text(encoding="utf-8")[:400]
        for needed in ('viewBox="0 0 24 24"', 'stroke="currentColor"', 'stroke-width="2"'):
            if needed not in head:
                print(f"{rel} is missing {needed}. The rail reads as one set only while every glyph"
                      " shares the delivered geometry.", file=sys.stderr)
                return 1

    text = render()
    if args.check:
        current = OUT.read_text(encoding="utf-8") if OUT.exists() else ""
        if current != text:
            print(f"{OUT.relative_to(ROOT)} is not what the tokens produce. Run"
                  " tools/generate_design_tokens.py.", file=sys.stderr)
            return 1
        count = len(re.findall(r"readonly property color", text))
        print(f"{OUT.relative_to(ROOT)} matches the vendored tokens ({count} colours)")
        return 0

    OUT.write_text(text, encoding="utf-8", newline="\n")
    count = len(re.findall(r"readonly property color", text))
    print(f"wrote {OUT.relative_to(ROOT)} with {count} colours from the vendored tokens")
    return 0


if __name__ == "__main__":
    sys.exit(main())
