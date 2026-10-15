#!/usr/bin/env bash
#
# Takes upstream work into one or every vendored subtree under crafts/.
#
# Redrob Canvas vendors the four upstream editors as git subtrees (UPSTREAM.md). Updating one is
# two commands that must run together -- the subtree pull, then the workspace regeneration -- and
# this script is the one place they are spelled out, so a pull is never left without its
# regenerated root Cargo.toml.
#
#   scripts/fetch-upstream.sh                      # every subtree, to the commit upstream-base.json records
#   scripts/fetch-upstream.sh photocraft           # one subtree, to its recorded commit
#   scripts/fetch-upstream.sh photocraft <sha>     # one subtree, to a NEW upstream commit
#
# After a pull to a new commit, record <sha> in upstream-base.json (base.commit and
# lastSyncedUpstream), docs/upstream-sources.toml (commit) and UPSTREAM_NOTICES.md, then run
# scripts/verify-upstream.sh. The merge this produces is a merge commit; keep it one when the branch
# lands (never squash a sync), or the subtree loses its merge base.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

pins="docs/upstream-sources.toml"
[ -f "$pins" ] || { echo "error: $pins not found" >&2; exit 1; }

read_pin() { # section key -> value
  python3 - "$pins" "$1" "$2" <<'PY'
import pathlib, sys, tomllib
path, section, key = sys.argv[1:4]
pins = tomllib.loads(pathlib.Path(path).read_text(encoding="utf-8"))
if section not in pins:
    sys.exit(f"section [{section}] not found in {path}")
if key not in pins[section]:
    sys.exit(f"{section}.{key} not found in {path}")
print(pins[section][key])
PY
}

code_sections() {
  python3 - "$pins" <<'PY'
import pathlib, sys, tomllib
pins = tomllib.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
print("\n".join(n for n, b in pins.items() if b.get("kind") == "code"))
PY
}

if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "error: the working tree has uncommitted changes; git subtree pull needs a clean tree" >&2
  exit 1
fi

targets=()
override_sha=""
case "$#" in
  0) mapfile -t targets < <(code_sections) ;;
  1) targets=("$1") ;;
  2) targets=("$1"); override_sha="$2" ;;
  *) echo "usage: $0 [<subtree> [<sha>]]" >&2; exit 2 ;;
esac

for name in "${targets[@]}"; do
  kind="$(read_pin "$name" kind)"
  if [ "$kind" != "code" ]; then
    echo "error: $name is kind=$kind, not a vendored subtree" >&2
    exit 1
  fi
  repo="$(read_pin "$name" repository)"
  prefix="$(read_pin "$name" prefix)"
  commit="${override_sha:-$(read_pin "$name" commit)}"
  [[ "$commit" =~ ^[0-9a-f]{40}$ ]] || { echo "error: $commit is not a full 40-character sha" >&2; exit 1; }

  # Already there? The newest squash commit for this prefix names the commit it carries.
  have="$(git log --all --format=%s --grep "^Squashed '$prefix/' content from commit " \
          | sed -n "1s/.*from commit \([0-9a-f]*\)$/\1/p")"
  if [ -n "$have" ] && [[ "$commit" == "$have"* ]]; then
    echo "$name already at $commit"
    continue
  fi

  echo "pulling $name from $repo at $commit into $prefix"
  git subtree pull --prefix "$prefix" "$repo" "$commit" --squash -m \
    "chore(upstream): update $name to $commit"
done

echo "regenerating the root Cargo.toml from crafts/*/Cargo.upstream.toml"
# The pull brings upstream's Cargo.toml back; merge_workspaces.py parks it as Cargo.upstream.toml
# again and rewrites the root manifest, so both the rename and the root file are committed together.
python3 tools/merge_workspaces.py
git add -A -- Cargo.toml crafts/
if ! git diff --cached --quiet; then
  git commit -q -m "build: regenerate the workspace after the subtree pull"
  echo "committed the regenerated Cargo.toml and the parked upstream manifests"
fi
echo "done. Record the new commit(s) in upstream-base.json, $pins and UPSTREAM_NOTICES.md, then run scripts/verify-upstream.sh"
