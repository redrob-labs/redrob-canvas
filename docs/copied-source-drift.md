# Copied-source drift harness

Stage 2, second harness. For the twelve files under `crates/redrob-core/src/geometry` that are Graphite's code
taken under Apache-2.0.

## The problem it fixes

Each of those files carries a header naming its upstream path, the pinned commit, and every change made to it,
ending with:

> Use `git diff --ignore-all-space` to compare against upstream.

**Nothing ever executed that instruction.** `scripts/verify-upstream.sh` checks the pin, the attribution, the
licence and the declared boundary — and its own comments say that for `kind = "code"` the re-sync method is a
diff against the upstream file — but no code performed the diff. A copied file could drift from the original it
claims to be, or gain an undeclared change, and every existing check would still pass.

This is precisely why `code` and `translated` are separate kinds: for a translation there is no file to diff and
behaviour is all that can be compared, while here there is one. This harness is what makes the distinction pay.

## Why tokens and not lines

The first version used `diff -w`, as the headers name. That ignores whitespace *within* a line but not line
breaks, and every copied file was reformatted from Graphite's hard tabs and long lines to this project's
rustfmt. `intersection.rs` reported **366 changed lines that were almost entirely re-wrapping**, and a real
one-line change would have been invisible inside that noise. A check too noisy to detect what it is for is the
wrong check.

`normalise.py` drops comments — ours carry the Apache 4(b) notice we are *required* to add, which is not
behaviour — and emits one Rust token per line. What survives a diff is an identifier, a literal, an operator or
a visibility keyword. The same twelve files then read:

| File | Tokens differing | Of | What the header declares |
|---|---|---|---|
| `glam_ext` | **0** | 220 | nothing but formatting |
| `polynomial` | 2 | 1,231 | two rustfmt trailing commas |
| `spline` | 7 | 1,453 | paths, visibility |
| `offset_bezpath` | 8 | 1,136 | paths |
| `consts` | 9 | 44 | three `pub(crate)` raised to `pub` |
| `poisson_disk` | 11 | 2,607 | paths |
| `shapes` | 23 | 2,806 | paths, visibility |
| `util` | 35 | 387 | paths, visibility |
| `intersection` | 53 | 4,027 | paths, 12 items raised, test import |
| `bezpath_algorithms` | 90 | 5,440 | paths, one `warn!` dropped |
| `convert` | 5,524 | 5,652 | an extraction from a 769-line module |
| `misc` | 5,181 | 5,652 | an extraction from the same module |

`convert` and `misc` are large because both take a handful of items out of one 769-line upstream module, so the
diff is mostly the discarded rest — which is what their headers say.

### The audit this made possible

Every removed identifier in `bezpath_algorithms` was checked against the file: `poisson_disk_sample`,
`point_to_dvec2`, `pathseg_tangent`, `pathseg_self_intersections`, `util`, `poisson_disk` all survive, having
lost only their leading path segments. The only genuinely removed identifier is `warn`, which the header
declares. Every constant in `consts.rs` holds the upstream's value; the nine differing tokens are three
`(crate)` removals and nothing else.

## Running it

```bash
scripts/fetch-upstream.sh graphite          # once; upstream/ is gitignored
./tools/upstream-diff/run.sh --list         # each copied file and its declared upstream path
./tools/upstream-diff/run.sh                # regenerate and compare against recorded/
./tools/upstream-diff/run.sh --update       # re-record, for a pin bump or an approved change
```

`REDROB_GRAPHITE_CHECKOUT` overrides where the pinned tree is looked for.

## It never passes silently

Reverse-verified, all four behaving as they must:

| Provoked | Result |
|---|---|
| `MAX_COINCIDENT_POINT_DISTANCE` changed from `1e-7` to `1e-8` | `DRIFTED`, exit 1 |
| a comment added to a copied file | `unchanged` — comments are not behaviour |
| checkout directory absent | `FAILURE, not a pass`, exit 1, naming the fetch command |
| checkout at the wrong commit | exit 1, because a diff against the wrong commit proves nothing |

The pin is read from `docs/upstream-sources.toml` and compared against the checkout's `HEAD`, so a stale tree
cannot silently become the authority.

## Why this and not a Graphite render comparison

The plan's stage-2 table says "canvas vector — SVG render pixels — Graphite CLI or a browser render". Checked
rather than assumed: **`node-graph/graphene-cli` does exist** and can write `.svg`, `.png`, `.jpg` and `.gif`.

It is still the wrong comparison for what was taken. It consumes Graphite's own node-graph document, which this
product has no exporter for, and renders through the node graph that `docs/graphite-port-boundary.md` records as
deliberately NOT ported — the decision was "take the libraries, not the graph". An end-to-end pixel comparison
would therefore measure our scanline rasteriser against theirs and differ for reasons that are by design, while
saying nothing about the twelve files actually copied.

What was taken is the geometry, and for copied code the exact authority is the file itself. If an exporter to
Graphite's document format ever exists, the render comparison becomes meaningful and this note is where to
start; recorded as out of scope with its reason rather than as done.
