#!/usr/bin/env bash
#
# Verifies the upstream pins without downloading 753 MB.
#
# Two different things need checking and only one of them needs a checkout:
#
#   * That the pins are well-formed, that every vendored source declares a commit, and that
#     upstream/ can never be committed. These are cheap and run on every push.
#   * That the pinned commits still EXIST upstream. Also cheap -- `git ls-remote` asks the remote
#     about one object without transferring a tree -- and it catches the failure mode this repo was
#     exposed to: a force-pushed or garbage-collected upstream commit turns a documented pin into
#     something nobody can reproduce, and the only copy would be one hand-staged tree on one
#     machine.
#
# Pass --with-trees to additionally verify a local checkout matches its pin. Off by default so CI
# stays fast.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

pins="docs/upstream-sources.toml"
with_trees=0
[ "${1:-}" = "--with-trees" ] && with_trees=1

echo "=== verifying upstream pins ==="

# --- 1. upstream/ must be unignorable-into-git ------------------------------------
# The tree is 753 MB. Before this, upstream/ was untracked and NOT ignored, so a `git add -A` would
# have committed three quarters of a gigabyte of someone else's source. That is the single most
# likely accident in this repository and it is cheap to make impossible.
# The rule is written `/upstream/` with a trailing slash, which matches a DIRECTORY. So the query
# needs the trailing slash too -- `git check-ignore upstream` misses it when the directory does not
# exist locally, which is exactly the state a fresh clone is in and therefore the state this check
# has to work in.
if ! git check-ignore -q "upstream/" 2>/dev/null; then
  echo "error: upstream/ is not ignored by git" >&2
  echo "       It holds ~753 MB of Krita and GIMP source; 'git add -A' would commit it." >&2
  exit 1
fi
# And prove the rule covers what actually gets written into it, not just the directory name.
for probe in "upstream/krita/README.md" "upstream/gimp/app/main.c" "upstream/graphite/Cargo.toml"; do
  if ! git check-ignore -q "$probe" 2>/dev/null; then
    echo "error: $probe is not ignored, so upstream content could still be committed" >&2
    exit 1
  fi
done
echo "upstream/ is ignored, so it cannot be committed by accident"

# --- 2. every vendored source declares a resolvable pin --------------------------
python3 - "$pins" <<'PY'
import re, sys, pathlib
path = sys.argv[1]
text = open(path, encoding="utf-8").read()

sections = dict(re.findall(r"^\[(\w+)\]\s*$(.*?)(?=^\[|\Z)", text, re.M | re.S))
# A section the parser cannot see is worse than a malformed one, because nothing complains. Measured on
# 2026-09-30: `[lcms2]repository = "..."` put the header and its first key on one line, so this regex --
# which requires the header alone on its line -- skipped the whole section. Its kind, licence and
# version pin were never checked and it appeared in no report, while the file looked complete to a
# reader. The file was also not valid TOML at all, which no consumer would have survived. So every
# bracketed name in the file must resolve to a parsed section.
declared = re.findall(r"^\[(\w+)\]", text, re.M)
unparsed = [name for name in declared if name not in sections]
if unparsed:
    print(
        f"error: these sections are declared but could not be parsed: {', '.join(unparsed)}\n"
        f"       the section header must be alone on its line",
        file=sys.stderr,
    )
    sys.exit(1)
required = ("krita", "gimp", "graphite")
KINDS = ("code", "translated", "algorithm", "library", "protocol")
problems = []

def value(body, key):
    found = re.search(rf'^{key}\s*=\s*"([^"]+)"\s*$', body, re.M)
    return found.group(1) if found else None

# A key written as a TOML multi-line string is INVISIBLE to `value()` above, which reads a single line.
# That reads as "absent" and yields a confusing "declares no boundary" on an entry whose boundary is
# sitting right there -- the same shape as the lcms2 section whose header shared a line with its first
# key and so went wholly unchecked. Absent and unreadable are different facts and get different messages.
def multiline_keys(body):
    return re.findall(r'^([a-z_]+)\s*=\s*"""', body, re.M)


for name in required:
    body = sections.get(name)
    if body is None:
        problems.append(f"[{name}] section is missing")
        continue
    for key in ("repository", "commit"):
        got = value(body, key)
        if not got:
            problems.append(f"{name}.{key} is missing")
            continue
        if key == "commit" and not re.fullmatch(r"[0-9a-f]{40}", got):
            problems.append(f"{name}.commit is not a full 40-character sha: {got!r}")
        if key == "repository" and not got.startswith("https://"):
            problems.append(f"{name}.repository is not https: {got!r}")

# A licence note per vendored source, because 'audit each reused file' is the actual obligation and
# an entry that quietly loses it is the one nobody re-reads.
for name in required:
    body = sections.get(name) or ""
    if not re.search(r'^license\s*=\s*"', body, re.M):
        problems.append(f"{name} declares no license")

# `kind` separates source we COPY from source we only READ. Getting that wrong is the one mistake in
# this file that cannot be walked back after shipping, so it is checked rather than left to prose.
for name, body in sections.items():
    for key in multiline_keys(body):
        problems.append(
            f"{name}.{key} is a TOML multi-line string, which this checker reads as absent; "
            f"write it on one line so it is actually validated"
        )

for name, body in sections.items():
    kind = value(body, "kind")
    if kind is None:
        problems.append(f"{name} declares no kind; must be one of {', '.join(KINDS)}")
    elif kind not in KINDS:
        problems.append(f"{name}.kind is {kind!r}, not one of {', '.join(KINDS)}")

# Every `code` source needs an attribution entry, because copying creates a duty that reading does
# not. THIRD_PARTY_NOTICES.md cannot serve: it is generated from Cargo.lock and carries a
# do-not-edit header, so copied source is recorded by hand in UPSTREAM_NOTICES.md.
notices_path = pathlib.Path("UPSTREAM_NOTICES.md")
notices = notices_path.read_text(encoding="utf-8") if notices_path.exists() else ""
if not notices:
    problems.append("UPSTREAM_NOTICES.md is missing")
for name, body in sections.items():
    kind = value(body, "kind")
    # A TRANSLATION carries the same duties as a copy, and that is the whole reason this kind exists.
    #
    # Rewriting a C++ function as Rust is a derivative work in copyright -- the idea/expression line does
    # not put a line-by-line translation on the safe side of it. So `translated` is validated exactly like
    # `code`: attribution, the pin recorded, a declared boundary, an inbound-compatible licence.
    #
    # It is a separate kind rather than `code` because the two differ in what a reader must then DO. For
    # `code` the re-sync method is a diff against the upstream file; for `translated` there is no file to
    # diff and behaviour is the only thing comparable. Calling both `code` would send the next reader
    # looking for a diff that cannot exist.
    #
    # `algorithm` was Krita's previous classification and it said "no file is reused". True while the three
    # colour adapters were the only work; false the moment a translated file lands. A classification that
    # quietly goes stale is exactly how `library` became a lie in the query repository once plugin files
    # started shipping.
    if kind not in ("code", "translated"):
        continue
    duty = "copying" if kind == "code" else "translating"
    if not re.search(rf"^##\s+{re.escape(name)}\s*$", notices, re.M | re.I):
        problems.append(
            f"{name}.kind is {kind!r} but UPSTREAM_NOTICES.md has no '## {name}' section"
        )
    commit = value(body, "commit")
    if commit and commit not in notices:
        problems.append(
            f"{name} is pinned at {commit[:12]} but UPSTREAM_NOTICES.md does not record that commit"
        )
    # If you take from a project you must say where taking stops. The interesting failures are at
    # the edge: a renderer that would fight ours, or a directory under a licence we may not use.
    if not value(body, "boundary"):
        problems.append(
            f"{name}.kind is {kind!r} but it declares no boundary; say where {duty} stops"
        )
    licence = value(body, "license") or ""
    # Inbound direction only. This product is GPL-3.0-or-later, so it can absorb these; it cannot be
    # redistributed under them. A copyleft that is not GPL-compatible would make the repository
    # undistributable, and that is worth failing a check over.
    # `-only` is refused explicitly, ahead of the allow-list, because the allow-list cannot see it.
    # LGPL-2.0-only and LGPL-2.1-only convert to GPL *version 2*, and GPL-2.0-only cannot become
    # GPL-3.0 -- a one-way door in the direction we need to go. MEASURED in the Krita tree at this
    # pin: 59 of 4,478 files carry an `-only` licence, and a bare substring match on "LGPL" waved all
    # 59 through on the first pass of the survey script. None of the 59 is a translation target, but
    # the check has to hold for the ones nobody surveyed.
    #
    # Anchored to a VERSION NUMBER, which is how SPDX spells these: `GPL-2.0-only`, never a bare word.
    # The looser `[A-Za-z0-9.]-only` this started as matched the prose "algorithm-only" already sitting
    # in Krita's own licence field, and refused a correct entry -- the check firing on a free-text field
    # rather than on a licence identifier.
    if re.search(r"\d\.\d-only", licence) and not re.search(r"GPL-3\.0-only", licence):
        problems.append(
            f"{name}.kind is {kind!r} and its license {licence!r} names an '-only' variant; those "
            f"reach GPL version 2 at best and cannot be taken into GPL-3.0-or-later"
        )
    elif not re.search(r"Apache-2\.0|MIT|BSD|ISC|GPL-3\.0|LGPL|Zlib|MPL-2\.0|GPL-2\.0-or-later", licence):
        problems.append(
            f"{name}.kind is {kind!r} but its license {licence!r} is not recognised as inbound-"
            f"compatible with GPL-3.0-or-later; audit it by hand and widen this check deliberately"
        )

if problems:
    for p in problems:
        print(f"error: {p}", file=sys.stderr)
    sys.exit(1)

kinds = {n: value(b, "kind") for n, b in sections.items()}
# Every kind the schema accepts is printed. A kind that is validated and then not shown is one the next
# reader mistrusts, and worse, it is where a wrong entry hides: four library pins and one protocol pin
# went unprinted here, and the lcms2 section that no parser could see went unnoticed with them.
def named(kind):
    return ", ".join(sorted(n for n, k in kinds.items() if k == kind)) or "none"

print(f"pins well-formed: {len(sections)} sources")
print(f"  copied source (licence binds us): {named('code')}")
print(f"  translated (derivative work):     {named('translated')}")
print(f"  behaviour only (nothing copied):  {named('algorithm')}")
print(f"  linked, not copied:               {named('library')}")
print(f"  spoken, not copied:               {named('protocol')}")
PY

# --- 3. the pinned commits still exist upstream ---------------------------------
#
# EVERY section that declares a commit, not just the vendored pair. The section that actually went
# dangling was `redrob_code`, which this loop did not look at: it read `krita gimp` from a hardcoded
# list, so the one pin that stopped resolving was the one nobody checked.
#
# And it now checks that the COMMIT exists, not merely that the remote answers. The old check ran
# `git ls-remote <repo> <sha>`, which matches refs and never a commit in a branch's history, so it
# fell through to "remote reachable" and passed for every input including a sha that had been
# garbage collected. That is precisely the failure this script's header says it exists to catch, and
# it could not catch it. GitHub's commit endpoint answers 422 for a sha it does not have, needs no
# token, and costs one request per pin.
python3 - "$pins" <<'PY'
import json, re, sys, urllib.error, urllib.request

path = sys.argv[1]
text = open(path, encoding="utf-8").read()
sections = dict(re.findall(r"^\[(\w+)\]\s*$(.*?)(?=^\[|\Z)", text, re.M | re.S))
# A section the parser cannot see is worse than a malformed one, because nothing complains. Measured on
# 2026-09-30: `[lcms2]repository = "..."` put the header and its first key on one line, so this regex --
# which requires the header alone on its line -- skipped the whole section. Its kind, licence and
# version pin were never checked and it appeared in no report, while the file looked complete to a
# reader. The file was also not valid TOML at all, which no consumer would have survived. So every
# bracketed name in the file must resolve to a parsed section.
declared = re.findall(r"^\[(\w+)\]", text, re.M)
unparsed = [name for name in declared if name not in sections]
if unparsed:
    print(
        f"error: these sections are declared but could not be parsed: {', '.join(unparsed)}\n"
        f"       the section header must be alone on its line",
        file=sys.stderr,
    )
    sys.exit(1)

def value(body, key):
    found = re.search(rf'^{key}\s*=\s*"([^"]+)"\s*$', body, re.M)
    return found.group(1) if found else None

problems = []
checked = 0

for name, body in sections.items():
    repo = value(body, "repository")
    commit = value(body, "commit")
    if not repo or not commit:
        continue
    checked += 1
    label = f"  {name:<12} {commit[:12]} ... "
    match = re.fullmatch(r"https://github\.com/([^/]+)/([^/.]+)(?:\.git)?/?", repo)
    if not match:
        # Not a host with a cheap existence check. Say so rather than printing a reassurance that
        # was never earned.
        print(f"{label}skipped (no cheap existence check for {repo})")
        continue
    owner, project = match.groups()
    url = f"https://api.github.com/repos/{owner}/{project}/commits/{commit}"
    request = urllib.request.Request(url, headers={"Accept": "application/vnd.github+json"})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            json.load(response)
        print(f"{label}exists")
    except urllib.error.HTTPError as error:
        if error.code in (404, 422):
            print(f"{label}GONE")
            problems.append(
                f"{name}.commit {commit} no longer exists in {repo}. A pin nobody can resolve is not"
                " a pin: replace it with a commit a tag or branch still reaches."
            )
        elif error.code == 403:
            # Anonymous rate limit. Not a pin problem, and failing the push over it would be worse
            # than saying nothing.
            print(f"{label}unchecked (GitHub rate limit)")
        else:
            print(f"{label}unchecked (HTTP {error.code})")
    except OSError as error:
        print(f"{label}unchecked ({error})")

if not checked:
    problems.append("no section declares both a repository and a commit; the pins file may have moved")

if problems:
    for problem in problems:
        print(f"error: {problem}", file=sys.stderr)
    sys.exit(1)
PY

# --- 4. optionally, a local checkout matches its pin ----------------------------
if [ "$with_trees" -eq 1 ]; then
  for name in graphite krita gimp; do
    target="upstream/$name"
    if [ ! -d "$target" ]; then
      echo "error: $target is absent; run scripts/fetch-upstream.sh" >&2
      exit 1
    fi
    if [ ! -d "$target/.git" ]; then
      echo "error: $target has no git metadata, so it cannot be checked against the pin" >&2
      echo "       Re-fetch it with scripts/fetch-upstream.sh rather than staging it by hand." >&2
      exit 1
    fi
    want=$(python3 -c "
import re, sys
text = open('$pins', encoding='utf-8').read()
body = re.search(r'^\[$name\]\s*\$(.*?)(?=^\[|\Z)', text, re.M | re.S).group(1)
print(re.search(r'^commit\s*=\s*\"([^\"]+)\"\s*\$', body, re.M).group(1))
")
    have=$(git -C "$target" rev-parse HEAD)
    if [ "$want" != "$have" ]; then
      echo "error: $target is at $have but the pin says $want" >&2
      exit 1
    fi
    echo "$name checkout matches its pin"
  done
fi

echo "=== upstream pins verified ==="
