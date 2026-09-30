#!/usr/bin/env python3
"""Replace the colour literals in qml/Main.qml with bindings onto the generated tokens.

WHY THIS NEEDS A PARSER AND NOT A REGEX. In QML `color:` sets the ink of a `Label` and the fill of a
`Rectangle`, and those take opposite ends of the ramp. Classifying by property name alone puts body ink
on a panel. So the enclosing component is tracked on a brace stack and the role follows from the pair.

WHAT IS DELIBERATELY LEFT ALONE. Three window properties -- `gradientStartColor`, `gradientEndColor`
and `exportMatte` -- and four `TextField.text` values are the user's DOCUMENT, not this app's chrome:
the default gradient a drawing is filled with, the matte an export is flattened onto, and the hex a
colour field shows for editing. Tokenising those would change what the user's artwork looks like and
make a colour field display a name instead of a value. They keep their literals.
"""
from __future__ import annotations

import pathlib
import re
import sys

SRC = pathlib.Path("qml/Main.qml")
LITERAL = re.compile(r'"#[0-9a-fA-F]{3,8}"')

# Lines whose literal is content, not chrome. Matched on the line's text so a moved line still skips.
CONTENT_MARKERS = (
    "property color gradientStartColor",
    "property color gradientEndColor",
    "property color exportMatte",
    'text: "#ff000000"',
    'text: "#ff5378dc"',
    'text: "#ff20242a"',
    'vectorStrokeColor.text = "#ff20242a"',
)

# Components whose `color` is INK. Everything else that takes a colour is a surface.
INK_TYPES = {"Label", "Text", "TextField", "TextArea", "TextEdit", "TextInput", "Button", "ToolTip"}


def srgb_to_linear(c: float) -> float:
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def parse(lit: str) -> tuple[float, float, float, float]:
    h = lit.strip('"').lstrip("#")
    if len(h) == 8:  # QML writes #AARRGGBB
        a, h = int(h[0:2], 16) / 255, h[2:]
    elif len(h) == 4:
        a, h = int(h[0] * 2, 16) / 255, "".join(c * 2 for c in h[1:])
    else:
        a = 1.0
        if len(h) == 3:
            h = "".join(c * 2 for c in h)
    r, g, b = (int(h[i : i + 2], 16) / 255 for i in (0, 2, 4))
    return r, g, b, a


def luminance(r: float, g: float, b: float) -> float:
    return 0.2126 * srgb_to_linear(r) + 0.7152 * srgb_to_linear(g) + 0.0722 * srgb_to_linear(b)


def family(r: float, g: float, b: float) -> str:
    """The hue, but only when there is enough of it to mean something.

    Measured on this file: the real status and brand colours sit at chroma 0.39 and above (#f0c674
    amber 0.49, #61c48a green 0.39, #5d7ef7 highlight 0.60), while the selection tints and blue-greys
    sit between 0.10 and 0.24 (#3d4658 0.11, #43557f 0.24, #7f8da6 0.15). A 0.10 cut called every one
    of those tints "blue" and painted a selected row in bright status blue -- a selected layer that
    looked like a notification. 0.30 separates the two groups with room on both sides.
    """
    mx, mn = max(r, g, b), min(r, g, b)
    if mx - mn < 0.30:
        return "neutral"
    if mx == r:
        h = 60 * (((g - b) / (mx - mn)) % 6)
    elif mx == g:
        h = 60 * ((b - r) / (mx - mn) + 2)
    else:
        h = 60 * ((r - g) / (mx - mn) + 4)
    if h < 18 or h >= 330:
        return "red"
    if h < 45:
        return "orange"
    if h < 70:
        return "amber"
    if h < 165:
        return "green"
    if h < 200:
        return "teal"
    if h < 255:
        return "blue"
    return "violet"


STATUS = {"red": "statusDanger", "orange": "statusWarning", "amber": "statusWarning",
          "green": "statusSuccess", "teal": "statusInfo", "blue": "statusInfo",
          "violet": "actionPrimary"}

PALETTE = {
    "palette.window": "surfaceBase",
    "palette.windowText": "inkPrimary",
    "palette.base": "surfaceRaised",
    "palette.alternateBase": "surfaceSunken",
    "palette.text": "inkPrimary",
    "palette.button": "surfaceRaised",
    "palette.buttonText": "inkPrimary",
    "palette.highlight": "actionPrimary",
    "palette.highlightedText": "inkOnBrand",
}


def token_for(prop: str, enclosing: str, lit: str) -> str:
    if prop in PALETTE:
        return PALETTE[prop]
    r, g, b, a = parse(lit)
    lum, fam = luminance(r, g, b), family(r, g, b)
    if a < 0.35:
        fam = "neutral"

    if prop.endswith("border.color") or prop == "border.color":
        if fam != "neutral":
            return "focusRing" if fam == "blue" else STATUS[fam]
        # On a dark ground a brighter edge is the strong one.
        return "borderStrong" if lum > 0.055 else "borderSubtle"

    if fam != "neutral":
        return STATUS[fam]

    if enclosing in INK_TYPES:
        # Dark ground: the lighter the value, the more important the text.
        return ("inkPrimary" if lum > 0.55 else
                "inkSecondary" if lum > 0.24 else "inkMuted")
    return ("surfaceBase" if lum < 0.012 else
            "surfaceRaised" if lum < 0.020 else
            "surfaceSunken" if lum < 0.038 else "borderSubtle")


text = SRC.read_text()
lines = text.split("\n")
stack: list[str] = []
converted = 0
skipped = 0
report: list[str] = []

for index, line in enumerate(lines):
    # Track the enclosing component before rewriting, so `Label { color: ... }` on one line resolves
    # to Label rather than to whatever contained it.
    opens = re.findall(r"\b([A-Z][A-Za-z0-9_.]*)\s*\{", line)
    enclosing = opens[-1] if opens else (stack[-1] if stack else "")

    if LITERAL.search(line):
        if any(marker in line for marker in CONTENT_MARKERS):
            skipped += len(LITERAL.findall(line))
            report.append(f"  keep  {index + 1:5} {line.strip()[:84]}")
        else:
            prop_match = re.findall(r"([A-Za-z_][\w.]*)\s*:\s*(?=[^:]*\"#)", line)
            prop = prop_match[-1] if prop_match else ""

            def repl(m: re.Match[str], _p: str = prop, _e: str = enclosing) -> str:
                global converted
                converted += 1
                return f"tokens.{token_for(_p, _e, m.group(0))}"

            new_line = LITERAL.sub(repl, line)
            if new_line != line:
                lines[index] = new_line

    for opened in opens:
        stack.append(opened)
    depth_change = line.count("{") - line.count("}")
    if depth_change < 0:
        del stack[depth_change:]

SRC.write_text("\n".join(lines))
for r in report:
    print(r)
print(f"\nbound {converted} literals to tokens, kept {skipped} that are the user's document")
left = LITERAL.findall(SRC.read_text())
if left:
    print(f"{len(left)} literals remain:", sorted(set(left)))
sys.exit(0)
