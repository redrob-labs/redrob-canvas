# What the colour bridges removed from the Krita port

Item 1a.4 of the porting plan: re-measure Krita's `libs/pigment` now that babl,
lcms2 and GEGL are wired, so the Krita translation work is sized against the
adapters that exist rather than against a guess.

Reproduce with `research/pigment-remains.py` in the workspace, against a sparse
checkout of `libs/pigment` at the pinned commit
`fdbf33b2146735465bb8aa59928fbc1890ceb160`.

## The headline

**`libs/pigment` does not need to be ported. Not one file.**

223 source files, 44,203 lines, and every one of them falls into one of two
categories:

| | Files | Lines | Share |
|---|---|---|---|
| Superseded by a wired adapter | 141 | 22,444 | 50.8% |
| Not ours to port at all | 82 | 21,759 | 49.2% |

Nothing is left over. That was not the expected answer — the plan said "much of
`libs/pigment` is what babl + lcms2 already do", which implied a remainder — so
the interesting part is why there is none.

## Per subsystem

| Group | Files | Lines | Superseded by |
|---|---|---|---|
| composite operations | 37 | 7,654 | GEGL, partly |
| resources: gradients, patterns, palettes | 20 | 7,797 | nothing — not colour management |
| their tests and benchmarks | 39 | 7,265 | nothing — we need our own |
| encoding traits, depth maths, scalers | 29 | 5,746 | babl |
| the `KoColorSpace` interface and `KoColor` | 7 | 3,240 | nothing — a design task, not a port |
| conversion graph and cache | 16 | 3,053 | babl |
| colour space registry and plugin architecture | 12 | 2,496 | nothing — we have no plugins |
| concrete simple colour spaces | 12 | 1,673 | babl |
| per-pixel operations over a region | 18 | 1,468 | GEGL, partly |
| ICC profile abstraction | 8 | 1,434 | lcms2 |
| histograms | 4 | 961 | nothing — a product feature |
| dithering | 9 | 832 | GEGL |
| conversion helpers and debug plumbing | 10 | 509 | babl |
| soft proofing | 2 | 75 | lcms2 |

## The measurement that changed the shape of the answer

**Zero files in `libs/pigment` include `lcms2.h`.**

That was found by grepping the checkout, and it is the fact everything else
follows from. `libs/pigment` is not Krita's colour *engine* — it is the
*abstraction* over one. The concrete ICC engine is
`plugins/color/lcms2engine`, **75 files**, and that is what our lcms2 adapter
replaces: 324 lines of C against 75 files of C++.

A first pass at this measurement classified by regex over file contents and
produced percentages that looked precise and were not — `/composite|blend/`
matches any file that merely mentions blending, and the lcms2 pattern was matching
string literals rather than includes. The zero-includes finding is what exposed
it. The script was rewritten to classify by name family, which is what actually
identifies a subsystem here, and every group in the table above states its
reasoning.

## Why there is no remainder, group by group

**babl subsumes more than format conversion.** `KoColorConversionSystem` is a
graph search for a path between two colour spaces, with a cache. That is
`babl_fish`, exactly — including the caching. 16 files implementing by hand what
one library call does. Add the per-model traits headers (RGB, BGR, CMYK, XYZ, Lab,
Gray), the depth maths, the u8↔u16 scalers, the transfer functions, and the four
concrete simple colour spaces, and babl accounts for 67 files and 10,981 lines.

**lcms2's share is small because pigment barely does ICC.** 10 files, 1,509 lines
— the profile abstraction and soft proofing. The engine it abstracts lives in the
plugin. What our adapter does *not* cover is a registry of loaded profiles, which
is ours to write and is small; and soft proofing needs the transform builder to
take a rendering intent, which it currently does not.

**GEGL's share is the least certain thing in this document.** 64 files and 9,954
lines of composite operations, per-pixel operations and dithering. GEGL has blend
operations and dithering in its 205-operation catalogue, but Krita's composite set
is its own, and our seam currently offers three parameterless operations. **The
real overlap is unmeasured until golden output compares them**, which is stage 2
and 3 of the porting plan. This is marked "partly" in the table and it is a
judgement, not a measurement.

**Half of it was never colour management.** `resources/` is gradients, patterns,
palettes and swatches — 20 files, 7,797 lines of document resource types that live
in the colour library for historical reasons. Histograms are a product feature.
Both belong to this product's own feature roadmap, and no adapter should cover
them.

**Krita's plugin architecture has nothing for us to port.** `KoColorSpaceRegistry`
and friends exist to discover colour spaces provided by plugins. This product has
no colour-space plugins, so there is nothing to discover. 12 files, 2,496 lines
that simply do not apply.

**The one genuine design task is the interface itself.** `KoColorSpace`,
`KoColorSpaceAbstract` and `KoColor` — 7 files, 3,240 lines — are Krita's own
abstraction shape. This product needs an equivalent, but writing one on top of
babl and lcms2 is a design task rather than a translation: most of what the Krita
interface exists to do is host plugins we do not have.

## What this does to stage 1-c

The porting plan's 1-c said: translate Krita, pigment first, in the order pigment
remainder → image → global/brush → paintops.

**The pigment step is removed.** Not reduced — removed. There is no remainder to
translate.

The first-order closure measured earlier was 2,018 of Krita's 5,978 files, of
which `libs/pigment` was 223. So 1-c drops to **1,795 files**, and its order
becomes `libs/image` (1,006) → `libs/global` + `libs/brush` (206) → `plugins/paintops`
(583).

Two consequences worth recording before that work starts:

1. The same question should be asked of `libs/image` before translating it. This
   measurement took one cycle and removed 223 files of planned work; assuming
   `libs/image` needs translating because it is in the closure would repeat the
   mistake this item was created to catch.

2. The composite-operation overlap is the largest open uncertainty and it sits on
   the critical path, because 64 files depend on the answer. It is settled by
   golden output, not by reading — which is one more reason stages 2 and 3 come
   before any of 1-c.
