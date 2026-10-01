#!/usr/bin/env python3
"""Emit one Rust token per line, so a diff shows real changes and not reformatting.

Why this exists. The first version of the copied-source harness used `diff -w`, which is what the file headers
themselves name (`git diff --ignore-all-space`). That ignores whitespace WITHIN a line but not line breaks, and
every copied file was reformatted from Graphite's hard tabs and long lines to this project's rustfmt settings.
So `intersection.rs` reported 366 changed lines that were almost entirely re-wrapping -- and a real one-line
semantic change would have been invisible inside that noise. A check too noisy to detect what it is for is the
wrong check.

Comparing token streams removes line breaking and indentation completely. What survives is what actually
changed: an identifier, a literal, an operator, a visibility keyword, a module path.

Comments are dropped. They carry the Apache 4(b) notice this repository is REQUIRED to add, and the upstream's
own prose, neither of which is behaviour. Doc comments go with them for the same reason.
"""

import re
import sys

# Rust tokens: identifiers and keywords, numbers (with suffixes and underscores), string and char literals,
# lifetimes, and multi-character operators before their single-character prefixes.
TOKEN = re.compile(
    r"""
      r\#*"(?:[^"\\]|\\.)*"\#*        # raw string
    | b?"(?:[^"\\]|\\.)*"             # string, byte string
    | b?'(?:[^'\\]|\\.)'              # char, byte char
    | '[A-Za-z_][A-Za-z0-9_]*         # lifetime
    | [0-9][0-9_]*\.[0-9_]*(?:[eE][+-]?[0-9_]+)?(?:f32|f64)?   # float
    | \.[0-9][0-9_]*                  # leading-dot float
    | 0[xXoObB][0-9a-fA-F_]+(?:[iu](?:8|16|32|64|128|size))?   # radix int
    | [0-9][0-9_]*(?:[iu](?:8|16|32|64|128|size)|f32|f64)?     # int
    | [A-Za-z_][A-Za-z0-9_]*          # identifier or keyword
    | \.\.=|\.\.\.|\.\.               # range
    | ::|->|=>|<<=|>>=|<<|>>          # paths, arrows, shifts
    | [<>=!+\-*/%^&|]=                # compound assignment and comparison
    | &&|\|\|                         # logical
    | \S                              # any other single character
    """,
    re.VERBOSE,
)


def strip_comments(text: str) -> str:
    """Remove line and block comments without breaking string literals that contain // or /*."""
    out = []
    index = 0
    length = len(text)
    while index < length:
        character = text[index]
        # A string or char literal is copied verbatim: a `//` inside one is not a comment.
        if character == '"' or character == "'":
            quote = character
            out.append(character)
            index += 1
            while index < length:
                if text[index] == "\\":
                    out.append(text[index : index + 2])
                    index += 2
                    continue
                out.append(text[index])
                if text[index] == quote:
                    index += 1
                    break
                index += 1
            continue
        if text.startswith("//", index):
            end = text.find("\n", index)
            index = length if end < 0 else end
            continue
        if text.startswith("/*", index):
            # Rust block comments nest.
            depth = 1
            index += 2
            while index < length and depth:
                if text.startswith("/*", index):
                    depth += 1
                    index += 2
                elif text.startswith("*/", index):
                    depth -= 1
                    index += 2
                else:
                    index += 1
            continue
        out.append(character)
        index += 1
    return "".join(out)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: normalise.py <file.rs>", file=sys.stderr)
        return 2
    try:
        text = open(sys.argv[1], encoding="utf-8").read()
    except OSError as error:
        print(f"cannot read {sys.argv[1]}: {error}", file=sys.stderr)
        return 1
    tokens = TOKEN.findall(strip_comments(text))
    if not tokens:
        # An empty token stream would make any comparison vacuously equal, which is the one result this must
        # never report silently.
        print(f"{sys.argv[1]} produced no tokens", file=sys.stderr)
        return 1
    sys.stdout.write("\n".join(tokens) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
