# Measuring interactive latency, and why the golden-output harness cannot

Item 1a.8 of the porting plan, created by what items 1a.5 and 1a.7 found.

## Baseline after 1c.2 block 8 — the projection is incremental

Taken at a load average of **1.48**, after the dirty-region projection landed. A dab now recomposites only the
region it damaged instead of the whole canvas.

```
        size  layers   megapixels      median       worst    of 60Hz
     300x300       1         0.09     0.155ms     0.182ms       0.0x
     800x800       1         0.64     1.017ms     1.274ms       0.1x
   1920x1080       1         2.07     3.474ms     3.901ms       0.2x
   4000x4000       1        16.00    35.896ms    37.445ms       2.2x
     800x800       4         0.64     1.031ms     1.183ms       0.1x
     800x800      16         0.64     1.102ms     1.177ms       0.1x
     800x800      40         0.64     1.234ms     1.378ms       0.1x
```

**The layer sweep is now flat**, which is what this document asked for in the first place: 1.017 ms at one
layer and 1.234 ms at forty. Before, forty layers cost 118.040 ms because every one of them was recomposited
for every dab.

The render half, measured separately with `cargo run --release -p redrob-core --example latency-split`:

| | render before | render after |
|---|---|---|
| 300×300 | 0.712 ms | 0.004 ms |
| 800×800 | 5.043 ms | 0.004 ms |
| 1920×1080 | 16.581 ms | 0.004 ms |
| 4000×4000 | 142.701 ms | **0.009 ms** |
| 800×800 × 40 | 119.264 ms | 0.048 ms |

### What the area sweep still costs is undo, not rendering

At 4000×4000 the dab is **35.896 ms**, and **35.6 ms of that is `Editor::execute_internal`**, which clones the
whole document three times per command to keep an undo snapshot (`before.clone()`, `after = before.clone()`,
`self.document = after.clone()`). One 4000×4000 layer is 64 MB, so a single dab moves about 190 MB.

That is an undo-architecture question — a journal of inverse operations, or a copy-on-write document with
per-cel sharing — and NOT the stroke scheduler. `RasterBytes` is already `Arc<Vec<u8>>`, so an unpainted layer
costs nothing to clone; the painted one is copied because it is written. Recorded here as the next bottleneck
rather than guessed at.

## The numbers are only comparable at a stated machine load

Discovered at 1c.2 block 6, and it applies to every table in this document. The dab placer was replaced and
the 4000×4000 median jumped from 178 ms to 210 ms — an 18% regression that would have been attributed to the
change. It was not the change.

The host's load average was **18** at that moment. Building the parent commit in a separate worktree and
running the same bench under the same load gave **214 ms** — the same as the new code, and 20% worse than
the same commit had measured an hour earlier at a quiet load.

**Confirmed at block 7**, one cycle later, at a load of **1.45**: 179.1 ms and 10.7× — the same figure the
tables below record, with both the spacing rewrite and the downscale filter in place. The 210 ms reading was
entirely the busy host.

So: **record the load average beside any figure taken from this bench, and compare only figures taken at
comparable load.** The earlier tables here were taken at a load near 2. The stage-3.5 submission must say
which, or the reviewer is given a number that cannot be checked.

Comparing against the parent commit in a throwaway worktree is the way to tell a real regression from a busy
machine, and costs a minute:

```
git worktree add -q /tmp/baseline HEAD~1
cd /tmp/baseline && cargo bench -p redrob-core
git worktree remove --force /tmp/baseline
```

## Re-measured at 1c.2 block 4, after the dab gained a shape

The tables below are the original baseline. The dab loop's inner arithmetic changed when the translated
brush dab shape replaced the fixed one-pixel feather, so the bench was re-run rather than assumed:

| size | layers | original | after the dab shape | change |
|---|---|---|---|---|
| 300×300 | 1 | 0.795 ms | 0.789 ms | −0.8% |
| 800×800 | 1 | 5.919 ms | 6.017 ms | +1.7% |
| 1920×1080 | 1 | 19.365 ms | 19.516 ms | +0.8% |
| 4000×4000 | 1 | 176.508 ms | 179.701 ms | +1.8% |
| 800×800 | 4 | 13.050 ms | 13.428 ms | +2.9% |
| 800×800 | 16 | 48.138 ms | 49.296 ms | +2.4% |
| 800×800 | 40 | 118.040 ms | 120.388 ms | +2.0% |

Every row is within 3% and no conclusion below changes: both sweeps are still linear, and the 4000×4000
figure quoted as **10.6× over the 60 Hz budget is now 10.8×**. The cost buys hardness, softness, aspect
ratio and image tips, replacing a subtraction and a clamp with a rational falloff and a per-dab mask.

This was caught only because `cargo clippy --all-targets` refused to compile the bench after
`Command::BrushStroke` gained a field. **`cargo test --workspace` does not build benches**, so a baseline
broken by an unrelated change would stay invisible until stage 3.5 asked for it. The bench belongs in the
gate set for that reason.


## The gap this closes

Stages 2 and 3 of the porting plan build a golden-output harness: render the same
document through this product and through Krita, compare the pixels. That harness
is the right instrument for almost everything measured so far.

It cannot see the largest genuinely missing subsystem.

Krita's stroke scheduler and dirty-region walkers are 8,910 lines across 50 files.
They change **no pixel**. They decide *which* pixels get recomputed when something
changes. A port validated purely by output parity would compare our canvas to
Krita's, find them identical, report green, and ship a product that recomputes the
entire canvas on every brush dab.

So this instrument was written **before** the work it measures, and its baseline was
recorded before any of that work began. `crates/redrob-core/benches/latency.rs`,
run with `cargo bench -p redrob-core`.

## What it measures

The time from a brush command being executed to a frame being available to present.
The render is inside the timed section deliberately — executing a command and never
presenting it is not latency a user experiences.

Two sweeps, and the shape matters more than the absolute numbers:

- **Area sweep** at one layer. Flat means only the touched region is recomputed.
  Growth proportional to megapixels means the whole canvas is.
- **Layer sweep** at one size. Flat means only the touched layer is composited.
  Growth means every layer is, whichever one was painted.

Median of 40 dabs rather than mean: one page fault in a hundred dabs moves a mean
and not a median, and the question is what a user feels per dab. One untimed dab
runs first so lazily allocated buffers are not charged to the measurement.

It asserts nothing. A latency budget that fails CI on a loaded build server is a
flaky test; the purpose is a recorded baseline to compare against.

## The baseline, measured 2026-09-30 before any scheduler work

```
        size  layers   megapixels      median       worst    of 60Hz
----------------------------------------------------------------------
     300x300       1         0.09     0.795ms     0.837ms       0.0x
     800x800       1         0.64     5.919ms     6.147ms       0.4x
   1920x1080       1         2.07    19.365ms    20.522ms       1.2x
   4000x4000       1        16.00   176.508ms   179.519ms      10.6x
     800x800       4         0.64    13.050ms    13.266ms       0.8x
     800x800      16         0.64    48.138ms    49.536ms       2.9x
     800x800      40         0.64   118.040ms   120.093ms       7.1x
```

**Both sweeps are linear. The whole canvas is recomputed on every dab, and every
layer is composited regardless of which one was painted.**

Per megapixel, across a 178-fold area range:

| Area | Median | Per megapixel |
|---|---|---|
| 0.09 MP | 0.795 ms | 8.83 ms/MP |
| 0.64 MP | 5.919 ms | 9.25 ms/MP |
| 2.07 MP | 19.365 ms | 9.36 ms/MP |
| 16.00 MP | 176.508 ms | 11.03 ms/MP |

A constant would mean the cost follows the region actually touched. It is constant
to within 25% across a 178× range, which is as clear a linear-in-area signal as this
kind of measurement produces.

Per layer, at a fixed 800×800:

| Layers | Median | Marginal cost |
|---|---|---|
| 1 | 5.919 ms | — |
| 4 | 13.050 ms | 2.38 ms/layer |
| 16 | 48.138 ms | 2.92 ms/layer |
| 40 | 118.040 ms | 2.91 ms/layer |

Fitted: **2.87 ms per layer** on a 3.04 ms base. Painting one layer costs the same
as painting all of them.

## What that means in a product, not a benchmark

A 60 Hz display gives 16.667 ms for everything. A 120 Hz stylus halves it.

- **A 1920×1080 document already misses the budget at a single layer** — 19.4 ms,
  1.2× over. That is a common canvas size, not a stress test.
- **A 4000×4000 document is 10.6× over** at 176 ms per dab. A stroke would arrive
  as roughly six dabs per second.
- **Sixteen layers at 800×800 is 2.9× over.** Sixteen layers is an ordinary
  illustration, not a pathological case.

None of this is a defect in the code that exists. redrob-core recomputes correctly
and the golden-output comparison will confirm that. It is the absence of a
subsystem, and the absence is invisible to every check the plan had before this
file.

## How this changes the plan

1. **Krita's `kis_latency_tracker` moves to the front of stage 1-c.** It is 576
   lines, it is the instrument Krita uses on itself, and translating it before the
   scheduler means the scheduler's effect is measurable as it lands rather than
   after.

2. **The scheduler stops being the last item by size and becomes a priority by
   measurement.** The translation order in `docs/krita-image-coverage.md` put it
   last because it is invisible to pixels. It is still invisible to pixels; it is
   now 10.6× over budget at a size users will actually open.

3. **Stage 3 needs a latency check beside the golden-output check.** A harness that
   reports pixel parity and says nothing about frame time will declare this product
   finished while it is unusable on a large canvas. The screenshot approval at stage
   3.5 will not catch it either: a screenshot of a correct canvas looks correct.

4. **This baseline is the comparison point.** Re-running `cargo bench -p
   redrob-core` after the scheduler lands should turn both sweeps flat. If it does
   not, the scheduler is not doing its job, and that is now a thing we can find out
   rather than a thing we would ship.
