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

## RE-MEASURED at 1c.2: 8,716 lines down to about 990

Rule applied from the previous block: measure against the CURRENT product before starting, because our
own earlier ports move the line. Here the reduction is not about our ports — it is about features this
product does not have.

| Krita | Lines | Verdict |
|---|---|---|
| `kis_cubic_curve.{h,cpp}`, `kis_cubic_curve_spline.h` | ~990 | **TRANSLATE** — the product has `Levels` and no way to express an arbitrary transfer curve |
| `kis_liquify_transform_worker.{h,cpp}` | 940 | no liquify tool exists |
| `kis_grid_interpolation_tools.h` | 995 | serves the mesh and warp transforms |
| `kis_warptransform_worker.{cc,h}` | 444 | no warp tool exists |
| `kis_perspectivetransform_worker`, `kis_perspective_math` | 458 | the product's transform is `Affine2D`; perspective is not affine and is not a feature yet |
| `kis_four_point_interpolator_{forward,backward}.h` | 355 | serves the transforms above |
| `kis_curve_{circle,rect}_mask_generator*` | 496 | brush masks — belongs to the brush block, not this one |
| `kis_polygonal_gradient_shape_strategy.{h,cpp}` | 443 | no polygonal gradient exists |

The transform group is the mesh finding again: correct mathematics with nothing to call it. Recorded with
its reason rather than as done — if a warp, liquify or perspective transform is ever specified, these
files are the authority and this table says where to restart.

### The tone curve, and why it was worth the cycle

The product ships ten filters, one of which is `Levels { input_black, input_white, gamma, output_black,
output_white }`. That expresses a monotonic remap with a single bend. It cannot express a curve that rises
and falls, which is what a tone-curve editor is for. `KisCubicCurve` is the authority, and it landed as
`crates/redrob-core/src/tone_curve.rs` behind a new `Filter::Curves { points }`.

**The algorithm departs from Krita's current one and that was measured, not assumed.** Krita assembles a
4n × 4n sparse system in the monomial basis and solves it with Eigen. This solves the same spline with the
tridiagonal (Thomas) algorithm in O(n) and needs no linear-algebra dependency — which the product does not
carry and would not be worth adding for a curve of a dozen points.

That substitution is only valid because a corner knot makes the system **separable**: a corner imposes a
zero second derivative on each side instead of matching derivatives, so each run between corners is an
independent natural spline. Compiling Krita's own Eigen assembly and comparing a five-point curve with an
interior corner against its two runs solved separately gives a maximum difference of **1.7e-15**. Every
expected value in the module's tests came from that binary, at twelve decimal places.

Krita's two clamps are both kept and both matter: `x` is clamped into the control range so the curve
extends flat rather than letting the outermost cubic run away, and `y` is clamped to [0, 1] because a
natural spline through points inside the unit square still overshoots between them.

## What has to be translated, largest first

| Group | Files | Lines | Why it is real work |
|---|---|---|---|
| ~~geometry, curves, interpolation maths~~ | ~~46~~ | ~~8,716~~ | **RE-MEASURED at 1c.2 — see below.** ~990 lines have a consumer; the rest is transform tooling this product does not have. |
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
