# What actually needs translating from Krita's libs/image

Item 1a.5 of the porting plan, and the direct consequence of 1a.4: being in the
dependency closure is not the same as needing translation. All 223 files of
`libs/pigment` turned out to be either superseded by a wired adapter or never
colour management to begin with. `libs/image` is 1,006 files by the closure's
count and 1,048 source files on disk, so the same question is worth a cycle
before any of it is translated.

Reproduce with `research/image-remains.py` in the workspace, against a sparse
checkout of `libs/image` at the pinned commit
`fdbf33b2146735465bb8aa59928fbc1890ceb160`.

## The answer

1,048 source files, 187,554 lines.

| Verdict | Files | Lines | Share |
|---|---|---|---|
| `not-ours` — their tests, their vendored third party | 296 | 62,287 | 33.2% |
| `ours` — redrob-core already implements it | 340 | 62,771 | 33.5% |
| **`TRANSLATE` — genuinely missing and genuinely theirs** | **224** | **43,687** | **23.3%** |
| `adapter` — babl, lcms2 or GEGL does this | 53 | 8,469 | 4.5% |
| unmatched — scattered small files, reviewed by hand | 135 | 10,340 | 5.5% |

Unlike `libs/pigment`, there IS a remainder here, and it is substantial: roughly
44,000 lines across 224 files, a quarter of the subsystem.

`libs/image` has a third axis `libs/pigment` did not. redrob-core already
implements a document, layers, selection, a raster surface, a render path, command
history, filters and the format adapters, in 9,680 lines of Rust — so a Krita file
can be superseded by our own code as well as by an adapter. That axis accounts for
a third of the subsystem on its own.

## What has to be translated, largest first

| Group | Files | Lines | Why it is real work |
|---|---|---|---|
| geometry, curves, interpolation maths | 46 | 8,716 | Pure maths, no Krita architecture. The easiest to translate and the most precisely checkable. |
| brush engine plumbing | 56 | 8,369 | The pressure and sensor model that feeds paintops. |
| lazybrush and flood fill | 23 | 7,503 | Colourize-mask lazy filling and flood fill. Real algorithms. |
| asynchronous stroke and update scheduler | 40 | 6,924 | Krita's concurrency model. redrob-core has none. |
| transform worker and perspective | 22 | 3,994 | Warp, cage, liquify, with their spline maths. |
| layer styles | 22 | 3,550 | Photoshop-compatible effects and their composition order. |
| dirty-region walkers and projection stores | 10 | 1,986 | Which region to recompute when a node changes. |
| ASL layer-style serialisation | 4 | 1,844 | A closed binary format Krita reverse-engineered. |
| random sources and noise salt | 1 | 801 | Deterministic RNG — see below. |

## Three findings that change how this work should be ordered

**1. A third of it is already ours, and that is a comparison job rather than a
port.** Layers and masks (64 files), iterators and the painter (32), undo and
commands (105), the tile engine (50), selection (10), animation (20) — all have
redrob-core equivalents already marked `native` in the compatibility matrix. The
work there is not writing code, it is proving the behaviour matches, which is
exactly what stages 2 and 3 build. Porting any of it would replace working code
with a translation.

The tile engine is the one judgement call in that group worth flagging. Krita
stores rasters as swappable pooled tiles; `raster.rs` does not. Tiling is a
*scaling* strategy rather than a behaviour — it changes memory and locality, not
output — so golden output cannot distinguish the two, which is precisely why it
can wait for a measurement that says it is needed.

**2. The largest genuinely missing subsystem is invisible to golden output.** The
stroke and update scheduler, plus the dirty-region walkers — 50 files, 8,910 lines
— are infrastructure: strokes decomposed into jobs, a scheduler, level-of-detail
previews, and the logic deciding which region to recompute. None of it appears in
a pixel comparison. A port sized and validated purely by output parity would
conclude it was unnecessary and ship an editor that recomputes the whole canvas on
every dab.

This matters for the plan's own order. Stages 2 and 3 build a golden-output
harness, and that harness will report green on a product missing this entirely.
Interactive latency needs its own measurement, and nothing in the current plan
provides one.

**3. The deterministic random sources are load-bearing, not incidental.**
`rand_salt.h` is one 801-line file and it would be easy to dismiss as noise
generation replaceable by any RNG. It is not: a scattering brush must scatter the
*same* way for its output to be comparable at all. Substituting a different RNG
makes every scatter-brush golden test permanently red for a reason that looks like
a bug in the brush.

## What this does to stage 1-c

After 1a.4 removed `libs/pigment`, the closure stood at 1,795 files. This
measurement does not remove `libs/image`, but it re-sizes it: of its 1,048 source
files, 224 need translating and 53 more are adapter work already partly done.

The revised shape of 1-c:

| Step | Closure files | Actually to translate |
|---|---|---|
| `libs/pigment` | 223 | **0** (measured in 1a.4) |
| `libs/image` | 1,006 | **224** |
| `libs/global` + `libs/brush` | 206 | not yet measured |
| `plugins/paintops` | 583 | not yet measured |

Two of the four are now measured and both came out far below their file count. The
remaining two deserve the same cycle each before translation starts — that is 789
files of assumption still standing.

Within `libs/image`, the order should follow verifiability rather than dependency,
because the maths is where golden output is most precise and the infrastructure is
where it says nothing:

1. geometry, curves, interpolation (8,716 lines) — checkable to the pixel
2. lazybrush and flood fill (7,503) — checkable to the pixel
3. transform worker (3,994) — checkable to the pixel
4. layer styles and ASL (5,394) — checkable, against a closed format
5. brush engine plumbing (8,369) — needs the paintops measurement first
6. stroke scheduler and walkers (8,910) — **needs a latency measurement, not a pixel one**
