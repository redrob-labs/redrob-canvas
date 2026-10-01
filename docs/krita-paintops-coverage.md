# What needs translating from plugins/paintops

Item 1a.7, the last of four subsystem measurements. Reproduce with
`research/krita-coverage.py paintops <checkout>` against a sparse checkout of
`plugins/paintops` at pinned commit
`fdbf33b2146735465bb8aa59928fbc1890ceb160`.

## The answer

586 source files, 44,921 lines.

| Verdict | Files | Lines | Share |
|---|---|---|---|
| **`TRANSLATE`** | **316** | **26,384** | **58.7%** |
| `not-ours` | 231 | 14,511 | 32.3% |
| `adapter` | 38 | 3,998 | 8.9% |
| unmatched | 1 | 28 | 0.1% |

The highest translation share of the four, which is what a brush engine should be:
nothing we have or link paints a stroke.

## The finding: MyPaint is a fourth bridge, not a port

**Krita does not reimplement MyPaint.** Its plugin includes
`<libmypaint/mypaint-brush.h>` and links the library. `libmypaint` 1.6.0 is
packaged with a pkg-config module and 14 public headers, so those 32 files are a
**bridge in the babl / lcms2 / GEGL pattern**, not translation work.

That is the same class of discovery as `libs/pigment` coming out at zero, and it
arrives the same way: by checking what the code actually includes instead of
assuming a directory named after a thing contains an implementation of it. The only
translation in the MyPaint group is the option data that configures the library.

libmypaint is added to `tools/fetch_optional_deps.sh` so the bridge can be built
and tested the way the other three were.

## What has to be translated

| Group | Files | Lines | Why |
|---|---|---|---|
| individual brush engines | 140 | 13,807 | Thirteen distinct behaviours. |
| option DATA: the parameter model | 107 | 6,654 | What a setting IS, not how it is edited. |
| dab generation and caching | 25 | 3,757 | One dab from a brush plus a sensor reading. |
| dynamic sensors and curve evaluation | 44 | 2,166 | Pressure, tilt, speed, angle, fade, distance. |

**The thirteen engines can land one at a time.** Smudge, spray, bristle, hatching,
deform, grid, tangent-normal, sketch, particle, curve, round marker, experiment,
and the default set — each is an independent behaviour, independently verifiable
against golden output. That makes this the most parallelisable block in the whole
port, and the only one where partial completion is a coherent product rather than a
half-finished subsystem.

**The sensors are the smallest group and the most load-bearing.** 2,166 lines
decide what makes a brush feel like that brush: how pressure maps to size, how
speed maps to opacity, how drawing angle rotates a dab. The compatibility matrix
lists a pressure-aware round brush as native and the full preset/sensor engine as
planned; this is the gap between those two rows.

## What is not ours: a third of it is UI for a toolkit we do not use

`libpaintop` is built as a **Data / Model / Widget triad per option**, and only one
third of that triad is behaviour:

| Layer | Files | Lines | Verdict |
|---|---|---|---|
| `*OptionData` — the values, ranges, defaults, serialisation | 107 | 6,654 | `TRANSLATE` |
| `*OptionModel` — lager reactive binding | 92 | 3,644 | `not-ours` |
| `*OptionWidget` + 41 `.ui` forms | 124 | 9,238 | `not-ours` |

The widgets are excluded for two independent reasons, either sufficient. This
product's interface is Qt Quick, not Qt Widgets, so the code does not apply. And
appearance is chrome governed by the Redrob design system — its colour tokens, its
two icon sizes, its 3:1 ink-on-ground contrast floor — so copying it would fail
`tools/check_design_system.py`.

This is the second and much larger instance of the category 1a.6 named:
**take the behaviour, reject the appearance.** There it was 4 files of canvas
handles; here it is 216 files and 12,882 lines, 29% of the subsystem. Worth stating
plainly because it is the single largest exclusion in the entire measurement, and a
port that copied it would arrive at a Krita-looking product that fails its own
design guard.

## All four subsystems, measured

| Subsystem | Closure files | Source files | To translate | Share | Item |
|---|---|---|---|---|---|
| `libs/pigment` | 223 | 223 | **0** | 0% | 1a.4 |
| `libs/image` | 1,006 | 1,048 | **224** | 23.3% | 1a.5 |
| `libs/global` + `libs/brush` | 206 | 207 | **79** | 49.4% | 1a.6 |
| `plugins/paintops` | 583 | 586 | **316** | 58.7% | 1a.7 |
| **total** | **2,018** | **2,064** | **619** | **30.7%** | |

The first-order dependency closure measured before any of this work was 2,018 of
Krita's 5,978 files. **619 files actually need translating — 30.7% of the closure,
and 10.4% of Krita.**

The pattern across the four is consistent and worth stating, because it predicts
where to look next: **a subsystem falls away in proportion to how much of its job
is already solved by something we have.** Colour management fell to zero because
three libraries do it. The document model fell to 23% because redrob-core has one.
Brush formats and geometry stayed at half because nothing we have reads an ABR
file or evaluates a Bezier patch. Paint engines stayed highest because nothing
paints.

## What the measurements changed about the plan

Four things, none of which were visible before the cycles that found them:

1. **`libs/pigment` was removed entirely** — 223 files of planned work that did not
   exist.
2. **libmypaint became a bridge** — 32 more files that are a pkg-config line.
3. **A measurement gap was found**: the stroke scheduler and dirty-region walkers,
   8,910 lines, are invisible to golden output, so the stage 2/3 harness will report
   green on a product missing them. Krita's own `kis_latency_tracker` closes it and
   is now first in the translation order.
4. **An exclusion category was found and named**: take the behaviour, reject the
   appearance. 220 files across two subsystems, and copying them would fail this
   product's own design guard.
