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

## RE-MEASURED at 1c.2, and the largest block almost vanished

The table below says the Bezier group is 9,165 lines — the largest single block of translation work found
in any subsystem. **That figure was measured against a product that no longer exists.** It was written at
item 1a.6, before the Graphite geometry port landed at 1b.1b and 1b.1c and brought kurbo 0.13, lyon_geom
1.0 and eighteen files of curve mathematics into `crates/redrob-core/src/geometry/`. That is the same
domain. The estimate did not go stale because Krita changed; it went stale because **this product did**.

Measured again at 1c.2, against the current tree:

| Krita's exported `KisBezierUtils` API | Where it already is |
|---|---|
| `linearizeCurve` | `kurbo::flatten` |
| `nearestPoint` | `ParamCurveNearest::nearest` |
| `curveLength` | `ParamCurveArclen::arclen` |
| `curveLengthAtPoint` | `subsegment` + `arclen` |
| `curveParamByProportion` | `inv_arclen` |
| `curveProportionByParam` | `subsegment` + `arclen` |
| `intersectWithLine` | `PathSeg::intersect_line` |
| `intersectWithLineNearest` | the minimum of the same set |
| `interpolateQuadric` | `QuadBez`, and `raise()` for the exact cubic |
| `controlPolygonZeros` | `geometry::polynomial` (Graphite port) |
| `offsetSegment` | `geometry::offset_bezpath` (Graphite port) |
| curve bounding rectangles | `Shape::bounding_box` |
| `mergeLinearizationSteps` | nothing — but it is a sorted merge, eight lines |
| `calculateLocalPos`, `calculateGlobalPos`, and their SVG2 variants | **nothing, and nothing needs them** |

Each row except the last two is **executed** in `crates/redrob-core/tests/krita_bezier_coverage.rs`
rather than asserted here, so a dependency bump that removes one of these behaviours fails a test
instead of quietly falsifying this table.

### The four remaining functions have no consumer

They take a `std::array<QPointF, 12>` — the twelve control points of a Bezier patch — and belong to
`KisBezierMesh`, which is Krita's free-form mesh transform and its SVG2 mesh gradient. Measured in this
repository: **zero occurrences** of `mesh` across every `.rs`, `.qml`, `.cpp` and `.h` file. This product
has `TransformActive { transform: Affine2D }`, an affine transform, and no mesh feature of any kind.

Translating 4,604 lines of mesh mathematics would produce code with nothing to call it — the same finding
as the query repository's read-only design leaving three of five Beekeeper roles with nothing to act on.
Recorded as out of scope **with its reason**, not as done: if a mesh transform is ever specified as a
product feature, `KisBezierMesh` is the authority and this measurement is where to restart.

### Two more things the product already settles differently

- `adjustIfOnPolygonBoundary` exists because Krita rasterises in floating point and must nudge a sample
  off an exact polygon edge. `semantic.rs` uses half-open scanline intervals (`a.y <= y < b.y`) with
  exact `i128` cross products, so the ambiguity has no way to arise. The workaround is not needed because
  the cause was avoided.
- The Qt type conversions in `kis_algebra_2d` (`toQPointF`, `fromQRect` and their kin) convert between Qt
  and Krita's own types. Neither side of that conversion exists here.

### What is genuinely missing, and is not urgent

`calculateConvexHull`, line clipping to a rect or convex polygon, and the rect helpers (`blowRect`,
`ensureInRect`, `alignRectToRect`, `findRectAnchor`). None has a consumer today. Worth noting that bounds
accumulation is currently hand-rolled in three places — `document::clipped_bounds`,
`semantic::clipped_fixed_bounds` and `semantic::edge_bounds` — which is a duplication to fix when
something needs a fourth, and a refactor rather than a translation.

## What has to be translated

| Group | Files | Lines | Why |
|---|---|---|---|
| ~~Bezier mesh and 2D algebra~~ | ~~22~~ | ~~9,165~~ | **SUPERSEDED — see the re-measurement above.** 13 of 17 exported functions are already in kurbo or the Graphite port; the other 4 are mesh-patch-only and the product has no mesh. |
| brush resource formats | 24 | 3,796 | ABR, GBR, GIH, PNG, SVG, text brushes. |
| brush model, generation, scaling | 23 | 3,743 | Brush value model, procedural generation, dab shaping, scaled-mask pyramid. |
| rolling statistics and latency tracking | 8 | 576 | Rolling means and a latency instrument. |
| deterministic random source | 2 | 159 | Must reproduce exactly — see below. |

~~**The Bezier group is the largest single block of translation work found so far, in
any subsystem** — 9,165 lines, more than any group in `libs/image`.~~

**Withdrawn at 1c.2.** The reasoning above was sound when written and the conclusion
is now wrong, because the Graphite geometry port arrived in between and covers the
same domain. The re-measurement at the top of this document replaces it. The order
below keeps its numbering so the progress log's references still resolve; step 2 is
now a measurement that produced a coverage test rather than a translation.

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
2. ~~**Bezier mesh and 2D algebra** (9,165)~~ — **re-measured to near zero at 1c.2.**
   Covered by kurbo and the Graphite port; the mesh remainder has no consumer.
3. geometry, curves, interpolation from `libs/image` (8,716) — same family
4. brush resource formats (3,796) — unobtainable elsewhere
5. lazybrush and flood fill (7,503)
6. brush model and generation (3,743) — after `plugins/paintops` is measured
7. transforms (3,994), layer styles and ASL (5,394)
8. stroke scheduler and walkers (8,910) — last, and validated by the tracker from
   step 1 rather than by pixels
