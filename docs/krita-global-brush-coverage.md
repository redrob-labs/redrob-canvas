# What needs translating from libs/global and libs/brush

Item 1a.6, the third of four subsystem measurements. Reproduce with
`research/krita-coverage.py global+brush <checkout>` against a sparse checkout of
`libs/global` and `libs/brush` at pinned commit
`fdbf33b2146735465bb8aa59928fbc1890ceb160`.

## The answer, and it runs the other way

207 source files (144 global + 63 brush), 35,286 lines.

| Verdict | Files | Lines | Share |
|---|---|---|---|
| **`TRANSLATE`** | **79** | **17,439** | **49.4%** |
| `not-ours` | 121 | 16,539 | 46.9% |
| `ours` | 7 | 1,308 | 3.7% |
| `adapter` | 0 | 0 | 0% |

The two previous measurements found most of a subsystem falling away — pigment
entirely, `libs/image` down to 23%. This one is the opposite: **half of it is real
work**, and the half that is not is not *our* half either.

That is not a surprise once the numbers are read. `libs/pigment` was colour
management, which three libraries already do. `libs/image` was a document model,
which redrob-core already has. These two are brush formats and geometry — and
nothing we have or link does either.

## What has to be translated

| Group | Files | Lines | Why |
|---|---|---|---|
| Bezier mesh and 2D algebra | 22 | 9,165 | Bezier patches, parameter-space sampling, region arithmetic. Pure maths. |
| brush resource formats | 24 | 3,796 | ABR, GBR, GIH, PNG, SVG, text brushes. |
| brush model, generation, scaling | 23 | 3,743 | Brush value model, procedural generation, dab shaping, scaled-mask pyramid. |
| rolling statistics and latency tracking | 8 | 576 | Rolling means and a latency instrument. |
| deterministic random source | 2 | 159 | Must reproduce exactly — see below. |

**The Bezier group is the largest single block of translation work found so far, in
any subsystem** — 9,165 lines, more than any group in `libs/image`. It is also the
cheapest to verify: pure maths, no Krita architecture, and golden output checks it
to the pixel. It belongs at the very front of 1-c for both reasons.

**The brush formats cannot be obtained any other way.** ABR is Photoshop's, GBR and
GIH are GIMP's, and the documentation for all three is thin. Krita's readers *are*
the specification available to us. This is the clearest case in the whole port for
copying rather than reimplementing, and the licence direction permits it.

**The latency tracker is the answer to a gap `libs/image` exposed.** Item 1a.5
found that the largest missing subsystem there — the stroke scheduler and
dirty-region walkers, 8,910 lines — is invisible to golden output, and that the
plan had no way to measure interactive latency. Krita instruments its own, in
`kis_latency_tracker` and the rolling-mean accumulators. 576 lines, and it closes
the measurement gap rather than adding a feature. It should be translated early,
before the work it exists to measure.

## What is not ours, and one case where copying would be a regression

`not-ours` is 121 files here, and unusually little of it is their tests (38).

**29 files of C++ language workarounds.** `KisCppQuirks`, `KisMpl`,
`KisPropagateConstWrapper`, `KisNewOnCopy`, the smart-pointer plumbing, the lock
adapters. These exist because C++ needs them. Rust's ownership model, `Arc`, and
the standard library are the equivalents — there is nothing to translate because
the problem does not occur.

**39 files of Qt signal and threading plumbing.** Signal compressors, acyclic
connectors, asserts, logging. Tied to Qt's conventions and to Krita's, not to any
behaviour.

**10 files of platform ports** — Krita's Android build and Windows Store packaging.

**And 4 files that would be an active regression to copy.**
`KisHandlePainterHelper` and `KisHandleStyle` draw the grab handles a transform
tool puts on the canvas, and style them. Handle appearance is *chrome*, and chrome
in this product is governed by the Redrob design system: its colour tokens, its two
icon sizes, its 3:1 ink-on-ground contrast floor. Copying Krita's styling here
would undo the design-system unification and fail
`tools/check_design_system.py`. This is the first place in the port where the
correct action is to take the *behaviour* and explicitly reject the *appearance*.

## Stage 1-c after three of four measurements

| Step | Closure files | Actually to translate | Measured in |
|---|---|---|---|
| `libs/pigment` | 223 | **0** | 1a.4 |
| `libs/image` | 1,006 | **224** | 1a.5 |
| `libs/global` + `libs/brush` | 206 | **79** | 1a.6 |
| `plugins/paintops` | 583 | ? | 1a.7 |

303 files of translation identified so far, against 1,435 closure files measured —
21%. One subsystem left, and it is the one the brush model above feeds into, so
its answer will interact with this one.

## Revised order for 1-c

Verifiability first, as `libs/image` argued, and now with a reason to move one item
to the front:

1. **latency tracker and rolling statistics** (576 lines) — it measures what
   nothing else can see, so it comes before the work it measures
2. **Bezier mesh and 2D algebra** (9,165) — largest block, most precisely checkable
3. geometry, curves, interpolation from `libs/image` (8,716) — same family
4. brush resource formats (3,796) — unobtainable elsewhere
5. lazybrush and flood fill (7,503)
6. brush model and generation (3,743) — after `plugins/paintops` is measured
7. transforms (3,994), layer styles and ASL (5,394)
8. stroke scheduler and walkers (8,910) — last, and validated by the tracker from
   step 1 rather than by pixels
