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
for probe in "upstream/krita/README.md" "upstream/gimp/app/main.c"; do
  if ! git check-ignore -q "$probe" 2>/dev/null; then
    echo "error: $probe is not ignored, so upstream content could still be committed" >&2
    exit 1
  fi
done
echo "upstream/ is ignored, so it cannot be committed by accident"

# --- 2. every vendored source declares a resolvable pin --------------------------
python3 - "$pins" <<'PY'
import re, sys
path = sys.argv[1]
text = open(path, encoding="utf-8").read()

sections = dict(re.findall(r"^\[(\w+)\]\s*$(.*?)(?=^\[|\Z)", text, re.M | re.S))
required = ("krita", "gimp")
problems = []

for name in required:
    body = sections.get(name)
    if body is None:
        problems.append(f"[{name}] section is missing")
        continue
    for key in ("repository", "commit"):
        found = re.search(rf'^{key}\s*=\s*"([^"]+)"\s*$', body, re.M)
        if not found:
            problems.append(f"{name}.{key} is missing")
            continue
        value = found.group(1)
        if key == "commit" and not re.fullmatch(r"[0-9a-f]{40}", value):
            problems.append(f"{name}.commit is not a full 40-character sha: {value!r}")
        if key == "repository" and not value.startswith("https://"):
            problems.append(f"{name}.repository is not https: {value!r}")

# A licence note per vendored source, because 'audit each reused file' is the actual obligation and
# an entry that quietly loses it is the one nobody re-reads.
for name in required:
    body = sections.get(name) or ""
    if not re.search(r'^license\s*=\s*"', body, re.M):
        problems.append(f"{name} declares no license")

if problems:
    for p in problems:
        print(f"error: {p}", file=sys.stderr)
    sys.exit(1)

print(f"pins well-formed: {', '.join(required)}")
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
  for name in krita gimp; do
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
