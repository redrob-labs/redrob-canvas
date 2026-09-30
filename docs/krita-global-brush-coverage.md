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

## RE-MEASURED at 1c.2 block 5: flood fill is the gap, lazybrush is not

Block 5 is lazybrush and flood fill, 7,503 lines. Measured against the product first, as the last three
blocks taught:

| Krita | Verdict |
|---|---|
| `libs/image/floodfill/kis_scanline_fill.cpp` (1,134) | **TRANSLATE** — the product had `Fill`, which paints the whole layer or selection, and no bucket tool at all |
| `libs/image/floodfill/kis_fill_interval_map.cpp` (162) | the interval bookkeeping the scanline walk needs |
| `libs/image/floodfill/kis_gap_map.cpp` (358) | fills across breaks in line art; a distinct feature, and there is no line-art mode here to justify it |
| `libs/image/lazybrush/*` (~5,800) | Krita's colourise mask: a multi-label optimisation over a whole layer, with no counterpart in this product |

Landing as `crates/redrob-core/src/flood_fill.rs` behind `Command::FloodFill { x, y, color, options }`.

### The tolerance is a Lab distance, and that was the expensive thing to get right

Krita compares colours with `differenceA`, which transforms both pixels to Lab through the document's ICC
profile and returns `sqrt(dL² + da² + db² + dAlpha²)` truncated to a byte. **Not an RGB distance**, and the
gap is not cosmetic: mid-grey 128 against 138 is 3 in Lab and 10 in RGB, so the same tolerance setting
selects visibly different pixels.

`Pixel` here is fixed sRGB with no profile, and this crate cannot reach the optional lcms2 adapter — that is
a C shim for the Qt layer, and a tolerance that changed with a build flag would be worse than a wrong one.
So the sRGB-to-Lab conversion is written out, with the Bradford-adapted D50 matrix an ICC connection space
uses rather than the D65 one.

**Verified against real lcms2 driving Krita's own transform**: exact agreement on all eight representative
pairs, and a maximum difference of 1 from rounding across 2,920 swept pairs.

The probe that produced those reference values was wrong first, and the thing that caught it was comparing
a colour against **itself**: it reported 34, because `TYPE_Lab_16` carries three channels and the fourth
alpha slot was never written. A self-comparison is the cheapest canary there is for reading uninitialised
output.

One guess of mine also failed against correct code. I expected a fixed sRGB step to be a larger Lab step in
the shadows than the highlights; measured, ΔL* for ten levels is **non-monotonic** — 2.74 at black, peaking
at 4.81 around level 32, falling to 3.46 at white. sRGB's encoding largely cancels Lab's cube root, which
is what the encoding is for, and its linear segment below 0.04045 compresses the deepest shadows.

## 1c.2 block 4: the brush formats need a model first

The plan's block 4 is the ABR, GBR and GIH readers — Photoshop's and GIMP's brush-tip formats, rightly
called unobtainable elsewhere. Measured before starting: **this product had no brush tip model at all.**

Its dab was a hard-coded circle with `(radius + 0.5 - distance).clamp(0, 1)` — a one-pixel linear feather,
no hardness, no aspect, no tip image. A format reader would have had nowhere to put a loaded tip, and a
reader whose output nothing consumes cannot be verified, only compiled.

1-c is ordered by verifiability rather than dependency, deliberately and for good reasons. That same
criterion is what moves the model ahead of the readers here.

So `libs/image/kis_circle_mask_generator.cpp` and the shared parts of `kis_base_mask_generator` were
translated instead, landing as `crates/redrob-core/src/dab_shape.rs` and reaching the product as
`BrushSettings::shape` — hardness, softness, aspect ratio and edge antialiasing.

Krita's falloff is `n(nf - 1)/(nf - n)`, a rational interpolation between the solid core and the rim,
neither linear nor gaussian. Every expected value in the module's tests is a byte printed by Krita's own
compiled `valueAt`. Findings from running it:

- **0 means opaque.** Krita's masks are inverted relative to coverage. First thing to get wrong.
- **Full hardness with antialiasing on is not perfectly hard.** The `+1.0` goes on both coordinates, so a
  sample on the x axis gets `y = 1.0` and the outermost pixel is feathered regardless of hardness. At
  diameter 40 that is 5 of 255 at x=19 and 125 at x=19.5.
- **Hardness 1.0 and softness 2.0 both collapse to a hard edge**, because each makes the fade coefficient
  equal the containment coefficient, so `nf == n` and every interior sample takes the opaque branch.
- **Hardness 0.0 and softness 0.1 give byte-identical masks at diameter 40.** Krita special-cases a zero
  fade to a coefficient of 1, and softness 0.1 lands on the same transformed value. Pinned by a test so
  the coincidence is not mistaken for a bug later.
- **Krita divides 0 by 0 at exactly the edge** when hardness is 1.0, and gets away with it only because
  casting the NaN to `quint8` happens to give 0 on x86.

One bug was introduced and caught while wiring this in. The mask was first resolved once per stroke from
`size`, ignoring that the dab radius is `size * pressure * 0.5`. A light-pressure dab then got the centre
of a full-size mask and came out flat at its own centre value with no falloff. Clippy surfaced it by
reporting `BrushDabRaster::radius` as never read once its only reader was replaced; the mask is now built
per dab and there is a regression test comparing two pressures normalised by their own centre values.

The readers themselves remain unchecked and are now unblocked: with a tip model in place, a loaded ABR or
GBR mask has somewhere to go. What they still need is a way to carry a tip IMAGE rather than a generated
shape, which is a larger piece of the same block.

### The tip image path, added at 1c.2 block 4's second half

With a dab model in place, the GBR reader has somewhere to put a loaded tip. `kis_gbr_brush.cpp` is
translated as `crates/redrob-core/src/brush_tip.rs`, and `Command::BrushStroke` gained
`tip: Option<BrushTip>` — an image tip that **replaces** the generated shape rather than multiplying with
it, since a tip already carries its own edge and multiplying would make every loaded brush softer than its
file says.

GBR was chosen over ABR and GIH because it is the self-contained one: 424 lines against ABR's 636 and the
image-pipe's 545, and neither of those adds a format so much as a container around this one. Findings from
compiling Krita's load path to generate the test fixtures:

- **A GBR byte is the coverage, because two inversions cancel.** GIMP stores 255 as paint, Krita stores
  `255 - v` for its own inverted masks, and this emits coverage. Byte 0 gives 0.00 and byte 255 gives 1.00.
- **Krita cannot read a small version-1 file.** It checks the 28-byte version-2 header length before
  reading the version, so a valid 23-byte version-1 brush is refused. This reads the version first.
- **Version 1 has no magic number and no spacing**, and its name therefore begins eight bytes earlier.
  Krita reads a version-1 name as Latin-1 and a version-2 name as UTF-8, because the older encoding was
  never defined.
- **Version 3 is CinePaint's and may hold float16 data.** Krita notes this in a comment and does not handle
  it; this refuses it by name rather than reading it as if it were version 2.
- **Spacing above 1000 is refused, and 1000 means a spacing of 10.**

The tip is carried BY VALUE on the command, bounded at 512 square by the decoder, the way vector paths and
gradient stops already are here. A resource store referencing tips by id would be the better home, and that
is architecture rather than translation, so it was not invented: recorded here as the next step instead.

**The latency baseline moved and that is recorded because it is a stage-3.5 deliverable.** The richer
falloff and the per-dab mask cost about 2%: the 4000×4000 single-layer median went from 176.5 ms to
179.7 ms, so the figure quoted as 10.6× of a 60 Hz budget is now 10.8×. Every row is within 3% and no
conclusion changes. Found only because `cargo clippy --all-targets` refused to compile the bench —
`cargo test --workspace` does not build benches, so a broken baseline would have stayed invisible until
stage 3.5 asked for it.

### ABR, added at 1c.2 block 4's third pass

Photoshop's format, and the one users actually have, because brush packs are everywhere. Its specification
is not published at all — Krita's reader and GIMP's before it are what exists. Unlike GIH it needs no new
concept: an ABR yields plain tips that plug into `BrushTip`.

`libs/brush/kis_abr_brush_collection.cpp` as `crates/redrob-core/src/abr.rs`, supporting exactly what
Krita's own `abr_supported_content` accepts — versions 1 and 2, and version 6 subversions 1 and 2 — plus
Photoshop's PackBits row compression. Fixtures are generated by `research/abr-fixtures.py`, which
reproduces both record layouts and Krita's `rle_decode` so the encoder and decoder are checked against each
other.

- **Krita loses every sampled brush after a computed one in a version 1 or 2 file.** It seeks
  `pos() + next_brush` past a computed brush, but `next_brush` is already absolute — its own other two
  exits seek plain `next_brush`, and the version 6 path does so everywhere. Demonstrated: the next record
  starts at byte 42 and Krita jumps to 52, landing inside it.
- **A depth other than 8 is refused**, where Krita would read a 16-bit brush at twice its size and render
  it as noise.
- **Short reads are errors.** Krita's `QDataStream` fails silently, so a truncated file gives a brush made
  of whatever was in the buffer.
- **PackBits' `-128` is a no-op that consumes no data byte**, and a row claiming more pixels than its width
  is truncated at the row rather than spilling into the next — both pinned by tests, because an unchecked
  `*data++` does neither.

### GIH is still blocked, and by a concept rather than by effort

The image-pipe format is GBR's data plus a **parasite**: a space-separated `key:value` string carrying
`dim`, `rank` and `sel`, describing a multi-dimensional selection among several tips. The product carries
one tip per stroke, so GIH needs a tip SET and a rule for choosing among them per dab — a brush-variation
feature, not a decoder. Recorded here rather than half-built.

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
