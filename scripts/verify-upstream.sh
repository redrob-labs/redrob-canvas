#!/usr/bin/env bash
#
# Verifies the upstream pins: the four upstream subtrees under crafts/ and the Redrob Code protocol
# pin. The rules it enforces are listed in UPSTREAM.md ("What the boundary guard enforces"); this
# header only says what it costs.
#
#   * Everything in steps 1 and 2 is local and runs in well under a second: it reads
#     upstream-base.json, docs/upstream-sources.toml, UPSTREAM_NOTICES.md, the attribution files,
#     and `git log` for each subtree's squash commit.
#   * Step 3 asks GitHub whether each pinned commit still exists and whether the redrob_code tag still
#     points at its commit: one request per pin, no token, no tree transferred. A force-pushed or
#     garbage-collected upstream commit turns a documented pin into something nobody can reproduce,
#     and that is the failure this step exists to catch. An anonymous rate limit is reported as
#     "unchecked", not as a failure.
#   * --with-trees additionally fetches each pinned commit (one shallow `git fetch` per subtree) and
#     checks that crafts/<name>/Cargo.upstream.toml is byte-identical to upstream's Cargo.toml at
#     that commit. Off by default so CI stays fast.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

with_trees=0
[ "${1:-}" = "--with-trees" ] && with_trees=1

echo "=== verifying upstream pins ==="

# --- 1 + 2. the pins are well-formed, agree with each other and with the tree ------------------
python3 - <<'PY'
import json, pathlib, re, subprocess, sys, tomllib

problems: list[str] = []

base = json.loads(pathlib.Path("upstream-base.json").read_text(encoding="utf-8"))
pins = tomllib.loads(pathlib.Path("docs/upstream-sources.toml").read_text(encoding="utf-8"))
notices = pathlib.Path("UPSTREAM_NOTICES.md").read_text(encoding="utf-8")

KINDS = ("code", "protocol")
# Inbound-compatible with MIT OR Apache-2.0. A copyleft licence is refused by not being here: the
# whole relicensing rests on no `code` entry ever being GPL or LGPL again.
PERMISSIVE = re.compile(r"\b(MIT|Apache-2\.0|BSD-[23]-Clause|ISC|Zlib|MPL-2\.0|Unicode-3\.0|CC0-1\.0)\b")
SHA = re.compile(r"[0-9a-f]{40}")

for name, body in pins.items():
    kind = body.get("kind")
    if kind not in KINDS:
        problems.append(f"{name}.kind is {kind!r}, not one of {', '.join(KINDS)}")
        continue
    repo = body.get("repository", "")
    if not repo.startswith("https://"):
        problems.append(f"{name}.repository is not https: {repo!r}")
    commit = body.get("commit", "")
    if not SHA.fullmatch(commit):
        problems.append(f"{name}.commit is not a full 40-character sha: {commit!r}")
    if kind != "code":
        continue
    for key in ("license", "boundary", "prefix"):
        if not body.get(key):
            problems.append(f"{name} is kind=code but declares no {key}")
    licence = body.get("license", "")
    if licence and not PERMISSIVE.search(licence):
        problems.append(
            f"{name}.license {licence!r} is not a permissive licence this MIT OR Apache-2.0 "
            f"repository can vendor; a copyleft input would make LICENSE-MIT a false statement"
        )
    if re.search(r"\bL?GPL\b", licence):
        problems.append(f"{name}.license {licence!r} names a copyleft licence; refused")
    # Attribution: a section and the pinned commit, by hand, in UPSTREAM_NOTICES.md.
    if not re.search(rf"^##\s+{re.escape(name)}\s*$", notices, re.M):
        problems.append(f"{name} is kind=code but UPSTREAM_NOTICES.md has no '## {name}' section")
    if commit and commit not in notices:
        problems.append(f"{name} is pinned at {commit[:12]} but UPSTREAM_NOTICES.md does not record it")

code_pins = {n: b for n, b in pins.items() if b.get("kind") == "code"}
base_entries = {e["name"]: e for e in base.get("upstreams", [])}

for name in sorted(set(code_pins) ^ set(base_entries)):
    where = "docs/upstream-sources.toml" if name in base_entries else "upstream-base.json"
    problems.append(f"{name} is in one pin file but missing from {where}")

for name in sorted(set(code_pins) & set(base_entries)):
    pin, entry = code_pins[name], base_entries[name]
    want = entry.get("base", {}).get("commit", "")
    if want != pin.get("commit"):
        problems.append(
            f"{name}: upstream-base.json says {want[:12]} but docs/upstream-sources.toml says "
            f"{pin.get('commit', '')[:12]}; the two must agree"
        )
    prefix = pin.get("prefix", "")
    if entry.get("prefix") != prefix:
        problems.append(f"{name}: prefix differs between the two pin files")
    if not pathlib.Path(prefix).is_dir():
        problems.append(f"{name}: {prefix} is not in the tree")
        continue
    # The newest squash commit for this prefix must name the pinned commit. git subtree writes the
    # sha at whatever abbreviation it chose (7 or more characters), so this is a prefix match.
    log = subprocess.run(
        ["git", "log", "--all", "--format=%s", "--grep", f"^Squashed '{prefix}/' content from commit "],
        capture_output=True, text=True, check=False,
    ).stdout.splitlines()
    squashed = [m.group(1) for line in log if (m := re.search(r"from commit ([0-9a-f]{7,40})$", line))]
    if not squashed:
        problems.append(f"{name}: no \"Squashed '{prefix}/' content from commit\" commit in git log")
    elif not want.startswith(squashed[0]):
        problems.append(
            f"{name}: the newest subtree squash commit names {squashed[0]}, the pin says {want[:12]}"
        )

# The attribution files must keep naming both holders; never replace an upstream notice.
for rule in base.get("attribution", []):
    path = pathlib.Path(rule["path"])
    if not path.is_file():
        problems.append(f"{rule['path']} is missing ({rule['why']})")
        continue
    text = path.read_text(encoding="utf-8")
    for needle in rule["mustContain"]:
        if needle not in text:
            problems.append(f"{rule['path']} no longer contains {needle!r}: {rule['why']}")

# Each upstream NOTICE must appear verbatim in ours: the obligation is the text, not a summary.
notice = pathlib.Path("NOTICE").read_text(encoding="utf-8") if pathlib.Path("NOTICE").is_file() else ""
for name, entry in base_entries.items():
    upstream_notice = pathlib.Path(entry["prefix"]) / "NOTICE"
    if upstream_notice.is_file() and upstream_notice.read_text(encoding="utf-8") not in notice:
        problems.append(f"NOTICE does not reproduce {upstream_notice} verbatim")

# Nothing of ours may reach into the trademarked brand assets.
excluded = tuple(base.get("excludedPrefixes", []))
if excluded:
    tracked = subprocess.run(["git", "ls-files", "-z"], capture_output=True, check=False).stdout.split(b"\0")
    allowed = {"UPSTREAM.md", "UPSTREAM_NOTICES.md", "NOTICE", "upstream-base.json",
               "docs/upstream-sources.toml", "docs/licensing.md", "scripts/verify-upstream.sh",
               "CHANGELOG.md"}
    for raw in tracked:
        rel = raw.decode("utf-8", "replace")
        if not rel or rel.startswith("crafts/") or rel in allowed:
            continue
        path = pathlib.Path(rel)
        if not path.is_file() or path.suffix.lower() in {".png", ".ico", ".icns", ".jpg", ".ttf", ".otf"}:
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        for prefix in excluded:
            if prefix in text or "artcraft-logo" in text or "artcraft-mark" in text:
                problems.append(f"{rel} references the upstream brand assets under {prefix}, which are trademarks")
                break

# redrob_code: the tag the pin records is the tag Cargo.toml depends on.
rc = pins.get("redrob_code", {})
cargo = pathlib.Path("Cargo.toml").read_text(encoding="utf-8")
m = re.search(r'redrob-engine-client\s*=\s*\{[^}]*tag\s*=\s*"([^"]+)"', cargo)
if rc and m and m.group(1) != rc.get("tag"):
    problems.append(
        f"redrob_code.tag is {rc.get('tag')} but Cargo.toml pins redrob-engine-client at {m.group(1)}"
    )

if problems:
    for p in problems:
        print(f"error: {p}", file=sys.stderr)
    sys.exit(1)

print(f"pins well-formed: {len(pins)} sources")
print(f"  vendored subtrees (licence binds us): {', '.join(sorted(code_pins)) or 'none'}")
print(f"  spoken, not copied:                   {', '.join(sorted(n for n, b in pins.items() if b['kind'] == 'protocol')) or 'none'}")
PY

# --- 3. the pinned commits still exist upstream -----------------------------------------------
#
# EVERY section that declares a commit. And it checks that the COMMIT exists, not merely that the
# remote answers: `git ls-remote <repo> <sha>` matches refs and never a commit in a branch's history,
# so it would pass for a sha that had been garbage collected. GitHub's commit endpoint answers 422
# for a sha it does not have, needs no token, and costs one request per pin.
python3 - <<'PY'
import json, pathlib, re, sys, tomllib, urllib.error, urllib.request

pins = tomllib.loads(pathlib.Path("docs/upstream-sources.toml").read_text(encoding="utf-8"))
problems = []
checked = 0

def api(url):
    request = urllib.request.Request(url, headers={"Accept": "application/vnd.github+json"})
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)

for name, body in pins.items():
    repo, commit = body.get("repository"), body.get("commit")
    if not repo or not commit:
        continue
    checked += 1
    label = f"  {name:<12} {commit[:12]} ... "
    match = re.fullmatch(r"https://github\.com/([^/]+)/([^/.]+)(?:\.git)?/?", repo)
    if not match:
        print(f"{label}skipped (no cheap existence check for {repo})")
        continue
    owner, project = match.groups()
    try:
        api(f"https://api.github.com/repos/{owner}/{project}/commits/{commit}")
        print(f"{label}exists")
    except urllib.error.HTTPError as error:
        if error.code in (404, 422):
            print(f"{label}GONE")
            problems.append(f"{name}.commit {commit} no longer exists in {repo}; re-pin to a reachable commit")
        elif error.code == 403:
            print(f"{label}unchecked (GitHub rate limit)")
        else:
            print(f"{label}unchecked (HTTP {error.code})")
    except OSError as error:
        print(f"{label}unchecked ({error})")

    tag = body.get("tag")
    if not tag:
        continue
    tag_label = f"  {name:<12} tag {tag} ... "
    try:
        ref = api(f"https://api.github.com/repos/{owner}/{project}/git/ref/tags/{tag}")
        target = ref["object"]["sha"]
        if ref["object"]["type"] == "tag":  # annotated: dereference one step
            target = api(f"https://api.github.com/repos/{owner}/{project}/git/tags/{target}")["object"]["sha"]
        if target == commit:
            print(f"{tag_label}points at the pinned commit")
        else:
            print(f"{tag_label}MOVED")
            problems.append(f"{name}.tag {tag} points at {target[:12]}, not the pinned commit {commit[:12]}")
    except urllib.error.HTTPError as error:
        if error.code == 404:
            print(f"{tag_label}GONE")
            problems.append(f"{name}.tag {tag} no longer exists in {repo}")
        elif error.code == 403:
            print(f"{tag_label}unchecked (GitHub rate limit)")
        else:
            print(f"{tag_label}unchecked (HTTP {error.code})")
    except OSError as error:
        print(f"{tag_label}unchecked ({error})")

if not checked:
    problems.append("no section declares both a repository and a commit; the pins file may have moved")
if problems:
    for problem in problems:
        print(f"error: {problem}", file=sys.stderr)
    sys.exit(1)
PY

# --- 4. optionally, each parked manifest is upstream's at the pin -------------------------------
if [ "$with_trees" -eq 1 ]; then
  python3 - <<'PY'
import pathlib, subprocess, sys, tempfile, tomllib

pins = tomllib.loads(pathlib.Path("docs/upstream-sources.toml").read_text(encoding="utf-8"))
failed = False
for name, body in pins.items():
    if body.get("kind") != "code":
        continue
    repo, commit, prefix = body["repository"], body["commit"], body["prefix"]
    parked = pathlib.Path(prefix) / "Cargo.upstream.toml"
    if not parked.is_file():
        print(f"error: {parked} is missing", file=sys.stderr); failed = True; continue
    with tempfile.TemporaryDirectory() as tmp:
        subprocess.run(["git", "init", "--quiet", tmp], check=True)
        subprocess.run(["git", "-C", tmp, "fetch", "--quiet", "--depth", "1", repo, commit], check=True)
        theirs = subprocess.run(["git", "-C", tmp, "show", f"{commit}:Cargo.toml"],
                                capture_output=True, check=True).stdout
    if theirs != parked.read_bytes():
        print(f"error: {parked} differs from {repo} Cargo.toml at {commit[:12]}", file=sys.stderr)
        failed = True
    else:
        print(f"{name}: Cargo.upstream.toml matches upstream at {commit[:12]}")
sys.exit(1 if failed else 0)
PY
fi

echo "=== upstream pins verified ==="
